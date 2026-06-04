//! Pragmatic C# source-level dependency extraction.
//!
//! C# `using` directives name namespaces, not files. The generic suffix
//! resolver cannot safely convert those namespaces into file edges. This pass
//! builds local type indexes from scanned `.cs` files, then emits dependency
//! edges only when a referenced type resolves unambiguously to another scanned
//! source file.

use crate::core::types::{FileNode, ImportEdge};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Clone, Debug)]
struct TypeDef {
    file: String,
}

#[derive(Default)]
struct TypeCatalog {
    by_full: HashMap<String, Vec<TypeDef>>,
    by_simple: HashMap<String, Vec<TypeDef>>,
}

#[derive(Default)]
struct FileContext {
    namespace: String,
    usings: Vec<String>,
    aliases: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Ident(String),
    Dot,
}

/// Build file-to-file dependency edges from C# type references.
pub(crate) fn build_csharp_reference_edges(
    files: &[&FileNode],
    scan_root: &Path,
) -> Vec<ImportEdge> {
    let csharp_files: Vec<&FileNode> = files
        .iter()
        .copied()
        .filter(|file| is_csharp_source(file))
        .collect();
    if csharp_files.is_empty() {
        return Vec::new();
    }

    let mut file_cache = HashMap::new();
    let mut contexts = HashMap::new();
    let catalog = build_type_catalog(&csharp_files, scan_root, &mut file_cache, &mut contexts);
    if catalog.by_simple.is_empty() {
        return Vec::new();
    }

    let mut edges = Vec::new();
    for file in csharp_files {
        let Some(content) = read_cached_content(file, scan_root, &mut file_cache) else {
            continue;
        };
        let ctx = contexts
            .entry(file.path.clone())
            .or_insert_with(|| parse_file_context(&content));
        let candidates = extract_reference_candidates(file, &content);
        for candidate in candidates {
            let Some(target) = resolve_candidate(&candidate, ctx, &catalog) else {
                continue;
            };
            if target != file.path {
                edges.push(ImportEdge {
                    from_file: file.path.clone(),
                    to_file: target,
                });
            }
        }
    }

    edges.sort_unstable_by(|a, b| {
        a.from_file
            .cmp(&b.from_file)
            .then_with(|| a.to_file.cmp(&b.to_file))
    });
    edges.dedup_by(|a, b| a.from_file == b.from_file && a.to_file == b.to_file);
    edges
}

pub(crate) fn is_csharp_source(file: &FileNode) -> bool {
    file.lang == "csharp" || file.path.to_ascii_lowercase().ends_with(".cs")
}

fn build_type_catalog(
    files: &[&FileNode],
    scan_root: &Path,
    file_cache: &mut HashMap<String, Option<String>>,
    contexts: &mut HashMap<String, FileContext>,
) -> TypeCatalog {
    let mut catalog = TypeCatalog::default();
    for file in files {
        let Some(content) = read_cached_content(file, scan_root, file_cache) else {
            continue;
        };
        let ctx = parse_file_context(&content);
        for simple in extract_declared_types(file, &content) {
            if !is_type_like_name(&simple) {
                continue;
            }
            let full = if ctx.namespace.is_empty() {
                simple.clone()
            } else {
                format!("{}.{}", ctx.namespace, simple)
            };
            let def = TypeDef {
                file: file.path.clone(),
            };
            catalog.by_full.entry(full).or_default().push(def.clone());
            catalog.by_simple.entry(simple).or_default().push(def);
        }
        contexts.insert(file.path.clone(), ctx);
    }
    catalog
}

fn read_cached_content(
    file: &FileNode,
    scan_root: &Path,
    file_cache: &mut HashMap<String, Option<String>>,
) -> Option<String> {
    if let Some(content) = file_cache.get(&file.path) {
        return content.clone();
    }
    let content = std::fs::read_to_string(scan_root.join(&file.path)).ok();
    file_cache.insert(file.path.clone(), content.clone());
    content
}

fn parse_file_context(content: &str) -> FileContext {
    let mut ctx = FileContext::default();
    for line in content.lines() {
        let line = remove_line_comment(line).trim();
        if let Some(namespace) = parse_namespace_line(line) {
            if ctx.namespace.is_empty() {
                ctx.namespace = namespace;
            }
            continue;
        }
        parse_using_line(line, &mut ctx);
    }
    ctx
}

fn parse_namespace_line(line: &str) -> Option<String> {
    let rest = line.strip_prefix("namespace ")?;
    let namespace = take_dotted_identifier(rest.trim_start());
    if namespace.is_empty() {
        None
    } else {
        Some(namespace)
    }
}

fn parse_using_line(line: &str, ctx: &mut FileContext) {
    let Some(rest) = line.strip_prefix("using ") else {
        return;
    };
    let rest = rest.trim_start();
    if rest.starts_with("static ") || rest.starts_with("var ") {
        return;
    }
    let rest = rest.trim_end_matches(';').trim();
    if let Some((alias, target)) = rest.split_once('=') {
        let alias = alias.trim();
        let target = take_dotted_identifier(target.trim());
        if is_identifier(alias) && !target.is_empty() {
            ctx.aliases.insert(alias.to_string(), target);
        }
        return;
    }
    let namespace = take_dotted_identifier(rest);
    if !namespace.is_empty() {
        ctx.usings.push(namespace);
    }
}

fn extract_declared_types(file: &FileNode, content: &str) -> HashSet<String> {
    let mut declared = HashSet::new();
    if let Some(classes) = file.sa.as_ref().and_then(|sa| sa.cls.as_ref()) {
        for class in classes {
            declared.insert(class.n.clone());
        }
    }

    let clean = strip_comments_and_strings(content);
    let tokens = tokenize_idents_and_dots(&clean);
    for i in 0..tokens.len() {
        let Some(word) = token_ident(&tokens, i) else {
            continue;
        };
        let name_index = match word {
            "class" | "interface" | "struct" | "enum" | "delegate" => {
                next_ident_index(&tokens, i + 1)
            }
            "record" => record_name_index(&tokens, i + 1),
            _ => None,
        };
        if let Some(name) = name_index.and_then(|idx| token_ident(&tokens, idx)) {
            if !is_csharp_keyword(name) {
                declared.insert(name.to_string());
            }
        }
    }
    declared
}

fn record_name_index(tokens: &[Token], start: usize) -> Option<usize> {
    let first = next_ident_index(tokens, start)?;
    match token_ident(tokens, first) {
        Some("class") | Some("struct") => next_ident_index(tokens, first + 1),
        _ => Some(first),
    }
}

fn extract_reference_candidates(file: &FileNode, content: &str) -> HashSet<String> {
    let mut candidates = HashSet::new();
    if let Some(sa) = &file.sa {
        if let Some(classes) = &sa.cls {
            for class in classes {
                if let Some(bases) = &class.b {
                    for base in bases {
                        insert_candidate(&mut candidates, base);
                    }
                }
            }
        }
        if let Some(calls) = &sa.co {
            for call in calls {
                insert_candidate(&mut candidates, call);
            }
        }
        if let Some(functions) = &sa.functions {
            for func in functions {
                if let Some(calls) = &func.co {
                    for call in calls {
                        insert_candidate(&mut candidates, call);
                    }
                }
            }
        }
    }

    let clean = blank_context_directives(&strip_comments_and_strings(content));
    let tokens = tokenize_idents_and_dots(&clean);
    for i in 0..tokens.len() {
        if let Some(word) = token_ident(&tokens, i) {
            insert_candidate(&mut candidates, word);
        }
        if let Some(chain) = dotted_chain_at(&tokens, i) {
            insert_candidate(&mut candidates, &chain);
        }
    }
    candidates
}

fn insert_candidate(candidates: &mut HashSet<String>, raw: &str) {
    let candidate = trim_type_candidate(raw);
    let last = candidate.rsplit('.').next().unwrap_or(&candidate);
    if is_type_like_name(last) && !is_csharp_keyword(last) {
        candidates.insert(candidate);
    }
}

fn resolve_candidate(candidate: &str, ctx: &FileContext, catalog: &TypeCatalog) -> Option<String> {
    let candidate = trim_type_candidate(candidate);
    if candidate.is_empty() {
        return None;
    }

    if let Some(alias_target) = ctx.aliases.get(&candidate) {
        return resolve_unique_full(alias_target, catalog);
    }

    if candidate.contains('.') {
        return resolve_unique_full(&candidate, catalog);
    }

    if !ctx.namespace.is_empty() {
        let same_namespace = format!("{}.{}", ctx.namespace, candidate);
        if let Some(target) = resolve_unique_full(&same_namespace, catalog) {
            return Some(target);
        }
    }

    for using_namespace in &ctx.usings {
        let full = format!("{}.{}", using_namespace, candidate);
        if let Some(target) = resolve_unique_full(&full, catalog) {
            return Some(target);
        }
    }

    resolve_unique_simple(&candidate, catalog)
}

fn resolve_unique_full(full_name: &str, catalog: &TypeCatalog) -> Option<String> {
    let defs = catalog.by_full.get(full_name)?;
    unique_file(defs)
}

fn resolve_unique_simple(simple_name: &str, catalog: &TypeCatalog) -> Option<String> {
    let defs = catalog.by_simple.get(simple_name)?;
    unique_file(defs)
}

fn unique_file(defs: &[TypeDef]) -> Option<String> {
    let mut unique = defs
        .iter()
        .map(|def| def.file.as_str())
        .collect::<HashSet<_>>();
    if unique.len() == 1 {
        unique.drain().next().map(str::to_string)
    } else {
        None
    }
}

fn blank_context_directives(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("using ") || trimmed.starts_with("namespace ") {
            out.push_str(&" ".repeat(line.len()));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

fn dotted_chain_at(tokens: &[Token], start: usize) -> Option<String> {
    let first = token_ident(tokens, start)?;
    let mut parts = vec![first.to_string()];
    let mut idx = start + 1;
    while matches!(tokens.get(idx), Some(Token::Dot)) {
        let Some(next) = token_ident(tokens, idx + 1) else {
            break;
        };
        parts.push(next.to_string());
        idx += 2;
    }
    if parts.len() > 1 {
        Some(parts.join("."))
    } else {
        None
    }
}

fn next_ident_index(tokens: &[Token], start: usize) -> Option<usize> {
    (start..tokens.len()).find(|&idx| matches!(tokens[idx], Token::Ident(_)))
}

fn token_ident(tokens: &[Token], index: usize) -> Option<&str> {
    match tokens.get(index) {
        Some(Token::Ident(value)) => Some(value.as_str()),
        _ => None,
    }
}

fn tokenize_idents_and_dots(content: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = content.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if is_ident_start(ch) {
            let mut end = start + ch.len_utf8();
            while let Some(&(idx, next)) = chars.peek() {
                if !is_ident_continue(next) {
                    break;
                }
                chars.next();
                end = idx + next.len_utf8();
            }
            tokens.push(Token::Ident(content[start..end].to_string()));
        } else if ch == '.' {
            tokens.push(Token::Dot);
        }
    }
    tokens
}

fn strip_comments_and_strings(content: &str) -> String {
    #[derive(Clone, Copy)]
    enum State {
        Normal,
        LineComment,
        BlockComment,
        String,
        VerbatimString,
        Char,
    }

    let mut state = State::Normal;
    let mut out = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();
    while let Some(ch) = chars.next() {
        match state {
            State::Normal if ch == '/' && chars.peek() == Some(&'/') => {
                chars.next();
                out.push_str("  ");
                state = State::LineComment;
            }
            State::Normal if ch == '/' && chars.peek() == Some(&'*') => {
                chars.next();
                out.push_str("  ");
                state = State::BlockComment;
            }
            State::Normal if ch == '@' && chars.peek() == Some(&'"') => {
                chars.next();
                out.push_str("  ");
                state = State::VerbatimString;
            }
            State::Normal if ch == '$' && chars.peek() == Some(&'"') => {
                chars.next();
                out.push_str("  ");
                state = State::String;
            }
            State::Normal if ch == '"' => {
                out.push(' ');
                state = State::String;
            }
            State::Normal if ch == '\'' => {
                out.push(' ');
                state = State::Char;
            }
            State::Normal => out.push(ch),
            State::LineComment if ch == '\n' => {
                out.push('\n');
                state = State::Normal;
            }
            State::LineComment => out.push(' '),
            State::BlockComment if ch == '*' && chars.peek() == Some(&'/') => {
                chars.next();
                out.push_str("  ");
                state = State::Normal;
            }
            State::BlockComment if ch == '\n' => out.push('\n'),
            State::BlockComment => out.push(' '),
            State::String if ch == '\\' => {
                out.push(' ');
                if chars.next().is_some() {
                    out.push(' ');
                }
            }
            State::String if ch == '"' => {
                out.push(' ');
                state = State::Normal;
            }
            State::String if ch == '\n' => out.push('\n'),
            State::String => out.push(' '),
            State::VerbatimString if ch == '"' && chars.peek() == Some(&'"') => {
                chars.next();
                out.push_str("  ");
            }
            State::VerbatimString if ch == '"' => {
                out.push(' ');
                state = State::Normal;
            }
            State::VerbatimString if ch == '\n' => out.push('\n'),
            State::VerbatimString => out.push(' '),
            State::Char if ch == '\\' => {
                out.push(' ');
                if chars.next().is_some() {
                    out.push(' ');
                }
            }
            State::Char if ch == '\'' => {
                out.push(' ');
                state = State::Normal;
            }
            State::Char if ch == '\n' => out.push('\n'),
            State::Char => out.push(' '),
        }
    }
    out
}

fn trim_type_candidate(raw: &str) -> String {
    raw.trim()
        .trim_start_matches("global::")
        .trim_matches(|ch: char| !is_ident_continue(ch) && ch != '.')
        .trim_end_matches('?')
        .to_string()
}

fn take_dotted_identifier(value: &str) -> String {
    value
        .chars()
        .take_while(|ch| is_ident_continue(*ch) || *ch == '.')
        .collect()
}

fn remove_line_comment(line: &str) -> &str {
    line.split_once("//").map(|(head, _)| head).unwrap_or(line)
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    is_ident_start(first) && chars.all(is_ident_continue)
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn is_type_like_name(value: &str) -> bool {
    value
        .chars()
        .next()
        .map(|ch| ch == '_' || ch.is_ascii_uppercase())
        .unwrap_or(false)
}

fn is_csharp_keyword(value: &str) -> bool {
    matches!(
        value,
        "abstract"
            | "as"
            | "base"
            | "bool"
            | "break"
            | "byte"
            | "case"
            | "catch"
            | "char"
            | "checked"
            | "class"
            | "const"
            | "continue"
            | "decimal"
            | "default"
            | "delegate"
            | "do"
            | "double"
            | "else"
            | "enum"
            | "event"
            | "explicit"
            | "extern"
            | "false"
            | "finally"
            | "fixed"
            | "float"
            | "for"
            | "foreach"
            | "goto"
            | "if"
            | "implicit"
            | "in"
            | "int"
            | "interface"
            | "internal"
            | "is"
            | "lock"
            | "long"
            | "namespace"
            | "new"
            | "null"
            | "object"
            | "operator"
            | "out"
            | "override"
            | "params"
            | "private"
            | "protected"
            | "public"
            | "readonly"
            | "record"
            | "ref"
            | "return"
            | "sbyte"
            | "sealed"
            | "short"
            | "sizeof"
            | "stackalloc"
            | "static"
            | "string"
            | "struct"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "uint"
            | "ulong"
            | "unchecked"
            | "unsafe"
            | "ushort"
            | "using"
            | "var"
            | "virtual"
            | "void"
            | "volatile"
            | "while"
            | "where"
            | "with"
            | "yield"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::test_helpers::make_file;

    #[test]
    fn namespace_only_using_does_not_create_edge() {
        let tmp = temp_root("namespace_only_using");
        std::fs::create_dir_all(tmp.join("src/App")).unwrap();
        std::fs::create_dir_all(tmp.join("src/Core")).unwrap();
        std::fs::write(
            tmp.join("src/App/Consumer.cs"),
            "using Ergon.Core;\nnamespace Ergon.App;\npublic sealed class Consumer { }\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("src/Core/Engine.cs"),
            "namespace Ergon.Core;\npublic sealed class Engine { }\n",
        )
        .unwrap();

        let files = vec![
            make_file("Consumer.cs", "src/App/Consumer.cs", "csharp", None),
            make_file("Engine.cs", "src/Core/Engine.cs", "csharp", None),
        ];
        let refs: Vec<&FileNode> = files.iter().collect();
        let edges = build_csharp_reference_edges(&refs, &tmp);
        let _ = std::fs::remove_dir_all(&tmp);

        assert!(
            edges.is_empty(),
            "using-only namespace import should not be a dependency edge"
        );
    }

    #[test]
    fn using_plus_type_reference_creates_edge() {
        let tmp = temp_root("using_type_reference");
        std::fs::create_dir_all(tmp.join("src/App")).unwrap();
        std::fs::create_dir_all(tmp.join("src/Core")).unwrap();
        std::fs::write(
            tmp.join("src/App/Consumer.cs"),
            "using Ergon.Core;\nnamespace Ergon.App;\npublic sealed class Consumer { public Engine Create() => new Engine(); }\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("src/Core/Engine.cs"),
            "namespace Ergon.Core;\npublic sealed class Engine { }\n",
        )
        .unwrap();

        let files = vec![
            make_file("Consumer.cs", "src/App/Consumer.cs", "csharp", None),
            make_file("Engine.cs", "src/Core/Engine.cs", "csharp", None),
        ];
        let refs: Vec<&FileNode> = files.iter().collect();
        let edges = build_csharp_reference_edges(&refs, &tmp);
        let _ = std::fs::remove_dir_all(&tmp);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from_file, "src/App/Consumer.cs");
        assert_eq!(edges[0].to_file, "src/Core/Engine.cs");
    }

    #[test]
    fn ambiguous_simple_type_requires_qualified_reference() {
        let tmp = temp_root("ambiguous_simple_type");
        std::fs::create_dir_all(tmp.join("src/App")).unwrap();
        std::fs::create_dir_all(tmp.join("src/Core")).unwrap();
        std::fs::create_dir_all(tmp.join("src/Other")).unwrap();
        std::fs::write(
            tmp.join("src/App/Consumer.cs"),
            "namespace Ergon.App;\npublic sealed class Consumer { public object Create() => new Ergon.Core.Engine(); }\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("src/Core/Engine.cs"),
            "namespace Ergon.Core;\npublic sealed class Engine { }\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("src/Other/Engine.cs"),
            "namespace Ergon.Other;\npublic sealed class Engine { }\n",
        )
        .unwrap();

        let files = vec![
            make_file("Consumer.cs", "src/App/Consumer.cs", "csharp", None),
            make_file("Engine.cs", "src/Core/Engine.cs", "csharp", None),
            make_file("Engine.cs", "src/Other/Engine.cs", "csharp", None),
        ];
        let refs: Vec<&FileNode> = files.iter().collect();
        let edges = build_csharp_reference_edges(&refs, &tmp);
        let _ = std::fs::remove_dir_all(&tmp);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to_file, "src/Core/Engine.cs");
    }

    fn temp_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "sentrux_csharp_refs_{}_{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        root
    }
}
