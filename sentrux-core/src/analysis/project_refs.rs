//! Project-file dependency extraction.
//!
//! Sentrux primarily builds dependency edges from source imports. Some ecosystems
//! also declare hard project references in build manifests. For .NET, a
//! `ProjectReference` is an architectural dependency even when the source-level
//! `using` directives are ambiguous, so we add those edges to the same
//! `ImportEdge` graph used by rules, DSM, cycles, and coupling metrics.

use crate::core::types::{FileNode, ImportEdge};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Build synthetic dependency edges from supported project manifests.
///
/// Edges are emitted from the referencing project file to the referenced
/// project file. Only references that resolve to files in the current scan are
/// emitted; external packages and missing files are deliberately ignored.
pub(crate) fn build_project_reference_edges(
    files: &[&FileNode],
    scan_root: &Path,
) -> Vec<ImportEdge> {
    let project_files = collect_project_files(files);
    if project_files.is_empty() {
        return Vec::new();
    }

    let mut edges = Vec::new();
    for project in project_files.values() {
        let abs_path = scan_root.join(project);
        let content = match std::fs::read_to_string(&abs_path) {
            Ok(content) => content,
            Err(_) => continue,
        };

        let project_dir = Path::new(project).parent().unwrap_or(Path::new(""));
        for include in extract_dotnet_project_references(&content) {
            let target = normalize_reference_path(project_dir, &include);
            let target_key = target.to_ascii_lowercase();
            if let Some(resolved) = project_files.get(&target_key) {
                if resolved != project {
                    edges.push(ImportEdge {
                        from_file: project.clone(),
                        to_file: resolved.clone(),
                    });
                }
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

fn collect_project_files(files: &[&FileNode]) -> HashMap<String, String> {
    let mut projects = HashMap::new();
    for file in files {
        if is_dotnet_project_file(&file.path) {
            projects.insert(file.path.to_ascii_lowercase(), file.path.clone());
        }
    }
    projects
}

fn is_dotnet_project_file(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    path.ends_with(".csproj") || path.ends_with(".fsproj") || path.ends_with(".vbproj")
}

fn extract_dotnet_project_references(content: &str) -> Vec<String> {
    let mut reader = Reader::from_str(content);
    let mut includes = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Empty(e)) | Ok(Event::Start(e))
                if e.name().as_ref().eq_ignore_ascii_case(b"ProjectReference") =>
            {
                for attr in e.attributes().flatten() {
                    if attr.key.as_ref().eq_ignore_ascii_case(b"Include") {
                        includes.push(String::from_utf8_lossy(attr.value.as_ref()).to_string());
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    includes
}

fn normalize_reference_path(project_dir: &Path, include: &str) -> String {
    let include = include.replace('\\', "/");
    let joined: PathBuf = project_dir.join(Path::new(&include));
    crate::analysis::resolver::suffix::normalize_path(&joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> FileNode {
        FileNode {
            path: path.to_string(),
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            is_dir: false,
            lines: 0,
            logic: 0,
            comments: 0,
            blanks: 0,
            funcs: 0,
            mtime: 0.0,
            gs: String::new(),
            lang: String::new(),
            sa: None,
            children: None,
        }
    }

    #[test]
    fn extracts_project_reference_include_attributes() {
        let refs = extract_dotnet_project_references(
            r#"
<Project>
  <ItemGroup>
    <ProjectReference Include="..\Core\Core.csproj" />
    <ProjectReference Include="../Domain/Domain.csproj"></ProjectReference>
  </ItemGroup>
</Project>
"#,
        );

        assert_eq!(refs.len(), 2);
        assert!(refs.contains(&"..\\Core\\Core.csproj".to_string()));
        assert!(refs.contains(&"../Domain/Domain.csproj".to_string()));
    }

    #[test]
    fn emits_edges_only_for_scanned_project_references() {
        let root = std::env::temp_dir().join(format!(
            "sentrux-dotnet-project-refs-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src/App")).unwrap();
        std::fs::create_dir_all(root.join("src/Core")).unwrap();
        std::fs::create_dir_all(root.join("src/Missing")).unwrap();
        std::fs::write(
            root.join("src/App/App.csproj"),
            r#"<Project><ItemGroup>
<ProjectReference Include="..\Core\Core.csproj" />
<ProjectReference Include="..\Missing\Missing.csproj" />
</ItemGroup></Project>"#,
        )
        .unwrap();
        std::fs::write(root.join("src/Core/Core.csproj"), "<Project />").unwrap();

        let files = vec![file("src/App/App.csproj"), file("src/Core/Core.csproj")];
        let refs: Vec<&FileNode> = files.iter().collect();

        let edges = build_project_reference_edges(&refs, &root);
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from_file, "src/App/App.csproj");
        assert_eq!(edges[0].to_file, "src/Core/Core.csproj");
    }
}
