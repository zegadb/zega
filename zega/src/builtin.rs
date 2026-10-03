//! Engine-owned types available in every schema.

pub const AUTH_SCHEMA: &str = r#"type Auth {
  uid: String
  email?: String
  emailVerified: Bool
  isAnonymous: Bool
  createdAt: Int
  name?: String
  avatarUrl?: String
}
unique { Auth { uid email } }
"#;

pub(crate) fn with_auth(source: &str) -> String {
    let (developer, extensions) = extract_extensions(source);
    if declares_auth_type(&developer) {
        return source.to_owned();
    }
    let mut auth = String::from(AUTH_SCHEMA);
    if !extensions.is_empty() {
        let close = auth
            .find("}\nunique")
            .expect("Auth schema has a type close");
        auth.insert_str(close, &extensions.join("\n"));
    }
    if developer.trim().is_empty() {
        auth
    } else {
        format!("{}\n{}", developer, auth)
    }
}

pub(crate) fn rejects_reserved(source: &str) -> bool {
    declares_auth_type(source) || unique_declares_auth(source)
}

fn declares_auth_type(source: &str) -> bool {
    source.match_indices("type Auth").any(|(start, matched)| {
        let before = &source[..start];
        let before_ok = (start == 0 || before.chars().next_back().is_some_and(|c| !c.is_alphanumeric() && c != '_'))
            && !before.trim_end().ends_with("extend");
        let end = start + matched.len();
        let after_ok = source[end..].chars().next().is_none_or(|c| c.is_whitespace() || c == '{');
        before_ok && after_ok
    })
}

fn unique_declares_auth(source: &str) -> bool {
    source.match_indices("unique").any(|(start, matched)| {
        let after = &source[start + matched.len()..];
        let Some(open) = after.find('{') else { return false };
        let mut depth = 0usize;
        let mut end = None;
        for (offset, ch) in after[open..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 { end = Some(open + offset + ch.len_utf8()); break; }
                }
                _ => {}
            }
        }
        end.is_some_and(|end| after[open..end].split(|c: char| !c.is_alphanumeric() && c != '_').any(|word| word == "Auth"))
    })
}

fn extract_extensions(source: &str) -> (String, Vec<String>) {
    let mut developer = String::new();
    let mut extensions = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.find("extend type Auth") {
        developer.push_str(&rest[..start]);
        let after = &rest[start..];
        let Some(open_rel) = after.find('{') else {
            developer.push_str(after);
            return (developer, extensions);
        };
        let body_start = open_rel + 1;
        let Some(end_rel) = after[body_start..].find('}') else {
            developer.push_str(after);
            return (developer, extensions);
        };
        let body = &after[body_start..body_start + end_rel];
        if body
            .lines()
            .filter(|line| !line.trim().is_empty())
            .all(|line| line.contains("->") || line.contains("<-"))
        {
            extensions.push(body.to_owned());
        } else {
            // Keep invalid extensions in developer text so the normal parser
            // reports the source error instead of silently accepting fields.
            developer.push_str(after);
            return (developer, extensions);
        }
        rest = &after[body_start + end_rel + 1..];
    }
    developer.push_str(rest);
    (developer, extensions)
}
