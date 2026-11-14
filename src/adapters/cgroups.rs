use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct ScopeSet {
    pub watched: Vec<PathBuf>,
    pub missing: Vec<String>,
}

/// Resolve the daemon's own cgroup v2 path from `/proc/self/cgroup`.
pub fn own_cgroup(root: &Path) -> Option<PathBuf> {
    let content = fs::read_to_string("/proc/self/cgroup").ok()?;
    let line = content.lines().find(|line| line.starts_with("0::"))?;
    let relative = line.strip_prefix("0::")?.trim();
    Some(root.join(relative.trim_start_matches('/')))
}

/// Resolve configured slice names under the cgroup root, excluding the
/// daemon's own cgroup and any ancestor of it.
pub fn discover(spec: &[String], root: &Path, own: &Option<PathBuf>) -> ScopeSet {
    let mut watched = Vec::new();
    let mut missing = Vec::new();

    for name in spec {
        let path = root.join(name);
        if !path.is_dir() {
            missing.push(name.clone());
            continue;
        }
        if let Some(own) = own
            && (path == *own || own.starts_with(&path))
        {
            continue;
        }
        watched.push(path);
    }

    ScopeSet { watched, missing }
}

pub fn slice_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::{discover, slice_name};

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("useless-cgroups-{}-{name}", std::process::id()))
    }

    #[test]
    fn discovers_existing_and_missing_slices() {
        let root = temp_root("discover");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("kubepods.slice")).expect("kubepods");
        fs::create_dir_all(root.join("user.slice")).expect("user");

        let spec = vec![
            "kubepods.slice".to_string(),
            "user.slice".to_string(),
            "missing.slice".to_string(),
        ];
        let scopes = discover(&spec, &root, &None);

        assert_eq!(scopes.watched.len(), 2);
        assert_eq!(scopes.missing, vec!["missing.slice".to_string()]);
        assert_eq!(slice_name(&scopes.watched[0]), "kubepods.slice");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn excludes_own_slice_ancestors() {
        let root = temp_root("own");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("kubepods.slice")).expect("kubepods");
        fs::create_dir_all(root.join("system.slice/useless.service")).expect("own");

        let own = Some(root.join("system.slice/useless.service"));
        let spec = vec!["kubepods.slice".to_string(), "system.slice".to_string()];
        let scopes = discover(&spec, &root, &own);

        assert_eq!(scopes.watched.len(), 1);
        assert!(scopes.watched[0].ends_with("kubepods.slice"));

        let _ = fs::remove_dir_all(&root);
    }
}
