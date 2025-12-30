use serde_json::Value;

#[derive(Debug, Default, Clone)]
pub struct SourceMap {
    /// Mapping from debuggee/DWARF paths to the client (VSCode) paths.
    target_to_client: Vec<(String, String)>,
    /// Reverse mapping from client (VSCode) paths to debuggee/DWARF paths.
    client_to_target: Vec<(String, String)>,
}

impl SourceMap {
    pub fn from_launch_args(arguments: &Value) -> Self {
        let mut sm = SourceMap::default();
        let Some(serde_json::Value::Object(map)) = arguments.get("sourceMap") else {
            return sm;
        };

        // Convention: key = target prefix, value = client prefix.
        for (target_prefix, client_prefix_val) in map.iter() {
            let Some(client_prefix) = client_prefix_val.as_str() else {
                continue;
            };

            let target_norm = Self::norm_prefix(target_prefix);
            let client_norm = Self::norm_prefix(client_prefix);

            sm.target_to_client
                .push((target_norm.clone(), client_prefix.to_string()));
            sm.client_to_target
                .push((client_norm, target_prefix.to_string()));
        }

        // Longest prefix wins.
        sm.target_to_client
            .sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        sm.client_to_target
            .sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        sm
    }

    pub fn map_target_to_client(&self, target_path: &str) -> String {
        self.apply_map(target_path, &self.target_to_client)
    }

    pub fn map_client_to_target(&self, client_path: &str) -> String {
        self.apply_map(client_path, &self.client_to_target)
    }

    fn apply_map(&self, path: &str, mapping: &[(String, String)]) -> String {
        let normalized = Self::norm_path(path);
        for (from_norm, to_raw) in mapping {
            if normalized.starts_with(from_norm) {
                let suffix = &normalized[from_norm.len()..];
                return Self::join_with_style(to_raw, suffix);
            }
        }
        path.to_string()
    }

    fn join_with_style(prefix: &str, suffix_norm: &str) -> String {
        if suffix_norm.is_empty() {
            return prefix.to_string();
        }
        let mut out = prefix.to_string();

        // Avoid double separators.
        let need_sep = !out.ends_with('/') && !out.ends_with('\\');
        if need_sep {
            // Pick separator style by prefix.
            out.push(if out.contains('\\') { '\\' } else { '/' });
        }

        let mut suffix = suffix_norm.to_string();
        // Convert suffix separators to match prefix style.
        if out.contains('\\') {
            suffix = suffix.replace('/', "\\");
        }
        out.push_str(&suffix);
        out
    }

    fn norm_prefix(s: &str) -> String {
        let mut out = Self::norm_path(s);
        if !out.ends_with('/') {
            out.push('/');
        }
        out
    }

    pub(crate) fn norm_path(s: &str) -> String {
        s.replace('\\', "/")
    }
}
