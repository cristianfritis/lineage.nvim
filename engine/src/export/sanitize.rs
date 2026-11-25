use std::path::Path;

use crate::cache::uri_to_path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedPath {
    pub display: String,
    pub is_external: bool,
}

#[derive(Debug, Clone)]
pub struct PathSanitizer {
    workspace_root_posix: Option<String>,
    absolute_paths: bool,
}

impl PathSanitizer {
    pub fn new(workspace_root: Option<&Path>, absolute_paths: bool) -> Self {
        let workspace_root_posix = workspace_root.and_then(|p| {
            let norm = normalize_posix_path(&p.to_string_lossy());
            if norm.is_empty() {
                None
            } else {
                Some(norm)
            }
        });
        Self {
            workspace_root_posix,
            absolute_paths,
        }
    }

    pub fn sanitize_uri(&self, uri_or_path: &str) -> SanitizedPath {
        let raw_path = if uri_or_path.starts_with("file://") {
            uri_to_path(uri_or_path).to_string_lossy().into_owned()
        } else {
            uri_or_path.to_string()
        };
        let norm = normalize_posix_path(&raw_path);

        if self.absolute_paths {
            let is_external = self
                .workspace_root_posix
                .as_deref()
                .map(|ws| !is_under_root(&norm, ws))
                .unwrap_or(false);
            return SanitizedPath {
                display: norm,
                is_external,
            };
        }

        if let Some(ws) = self.workspace_root_posix.as_deref() {
            if norm == ws {
                return SanitizedPath {
                    display: ".".to_string(),
                    is_external: false,
                };
            }
            if let Some(rel) = strip_root_prefix(&norm, ws) {
                return SanitizedPath {
                    display: rel.to_string(),
                    is_external: false,
                };
            }
        }

        SanitizedPath {
            display: external_tail(&norm),
            is_external: true,
        }
    }

    pub fn scrub_text(&self, text: &str) -> String {
        // 1. Replace embedded `file://...` URIs first using sanitize_uri.
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(pos) = rest.find("file://") {
            out.push_str(&rest[..pos]);
            let after = &rest[pos..];
            let end = after
                .find(|c: char| {
                    c.is_whitespace() || matches!(c, '\'' | '"' | '`' | ')' | ']' | '}' | ',' | ';')
                })
                .unwrap_or(after.len());
            let uri_slice = &after[..end];
            out.push_str(&self.sanitize_uri(uri_slice).display);
            rest = &after[end..];
        }
        out.push_str(rest);

        // 2. Strip explicit workspace_root prefix if present.
        if !self.absolute_paths {
            if let Some(ws) = self.workspace_root_posix.as_deref() {
                let ws_slash = format!("{ws}/");
                out = out.replace(&ws_slash, "");
                if out.contains(ws) {
                    out = out.replace(ws, ".");
                }
                let ws_back = ws.replace('/', "\\");
                let ws_back_slash = format!("{ws_back}\\");
                out = out.replace(&ws_back_slash, "");
            }
            // 3. Scrub raw host user home prefixes (/home/<user>/, /Users/<user>/, C:/Users/<user>/).
            out = scrub_host_prefixes(&out);
        }

        out
    }
}

fn normalize_posix_path(raw: &str) -> String {
    let slash = raw.replace('\\', "/");
    let has_leading_slash = slash.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for seg in slash.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if !parts.is_empty() && *parts.last().unwrap() != ".." {
                    parts.pop();
                } else if !has_leading_slash {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    if has_leading_slash {
        format!("/{}", parts.join("/"))
    } else {
        parts.join("/")
    }
}

fn is_under_root(path: &str, root: &str) -> bool {
    path == root || strip_root_prefix(path, root).is_some()
}

fn strip_root_prefix<'a>(path: &'a str, root: &str) -> Option<&'a str> {
    if root == "/" {
        return path.strip_prefix('/');
    }
    let rest = path.strip_prefix(root)?;
    rest.strip_prefix('/')
}

fn external_tail(norm_path: &str) -> String {
    let segs: Vec<&str> = norm_path
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    if segs.is_empty() {
        return "<external>".to_string();
    }

    // Strip leading host user home or drive prefixes.
    let mut start = 0usize;
    if segs.len() >= 2 && (segs[0] == "home" || segs[0] == "Users") {
        start = 2;
    } else if is_windows_drive(segs[0]) {
        start = 1;
        if segs.len() >= 3 && segs[1].eq_ignore_ascii_case("Users") {
            start = 3;
        }
    }
    let remaining = &segs[start.min(segs.len())..];
    if remaining.is_empty() {
        return "<external>".to_string();
    }

    // Keep at most the last 3 path segments for concise, privacy-safe external identification.
    let tail_start = remaining.len().saturating_sub(3);
    format!("<external>/{}", remaining[tail_start..].join("/"))
}

fn is_windows_drive(seg: &str) -> bool {
    let b = seg.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

fn scrub_host_prefixes(text: &str) -> String {
    let mut current = text.to_string();
    for prefix in ["/home/", "/Users/"] {
        let mut next = String::with_capacity(current.len());
        let mut rest = current.as_str();
        while let Some(idx) = rest.find(prefix) {
            next.push_str(&rest[..idx]);
            let after_prefix = &rest[idx + prefix.len()..];
            let user_end = after_prefix
                .find(|c: char| c == '/' || c.is_whitespace() || matches!(c, '"' | '\'' | '`'))
                .unwrap_or(after_prefix.len());
            let after_user = &after_prefix[user_end..];
            if let Some(stripped) = after_user.strip_prefix('/') {
                next.push_str("<external>/");
                rest = stripped;
            } else {
                next.push_str("<external>");
                rest = after_user;
            }
        }
        next.push_str(rest);
        current = next;
    }

    // Also scrub Windows C:\Users\<user>\ or C:/Users/<user>/
    let bytes = current.as_bytes();
    let mut out = String::with_capacity(current.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if i + 9 < bytes.len()
            && bytes[i].is_ascii_alphabetic()
            && bytes[i + 1] == b':'
            && (bytes[i + 2] == b'/' || bytes[i + 2] == b'\\')
        {
            let tail = &current[i + 3..];
            if tail.len() >= 6 && tail[..6].eq_ignore_ascii_case("Users/")
                || tail.len() >= 6 && tail[..6].eq_ignore_ascii_case("Users\\")
            {
                let after_users = &tail[6..];
                let user_end = after_users
                    .find(|c: char| {
                        c == '/' || c == '\\' || c.is_whitespace() || matches!(c, '"' | '\'' | '`')
                    })
                    .unwrap_or(after_users.len());
                let after_user = &after_users[user_end..];
                out.push_str("<external>/");
                let advance = 3
                    + 6
                    + user_end
                    + usize::from(after_user.starts_with('/') || after_user.starts_with('\\'));
                i += advance;
                continue;
            }
        }
        let ch = current[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}
