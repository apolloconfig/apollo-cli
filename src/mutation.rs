use serde::Serialize;

const MAX_TABLE_VALUE_CHARS: usize = 200;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationPlan {
    pub operation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<MutationScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<MutationScope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emergency: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_release_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_namespace: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub append_namespace_prefix: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<MutationRequest>,
}

impl MutationPlan {
    pub fn new(operation: impl Into<String>) -> Self {
        Self {
            operation: operation.into(),
            profile: None,
            server: None,
            source: None,
            target: None,
            key: None,
            key_count: None,
            release_title: None,
            emergency: None,
            release_id: None,
            to_release_id: None,
            public_namespace: None,
            append_namespace_prefix: None,
            request: None,
        }
    }

    pub fn with_context(mut self, profile: Option<String>, server: Option<&str>) -> Self {
        self.profile = profile;
        self.server = server.map(sanitize_server);
        self
    }

    pub fn with_source(mut self, source: MutationScope) -> Self {
        self.source = Some(source);
        self
    }

    pub fn with_target(mut self, target: MutationScope) -> Self {
        self.target = Some(target);
        self
    }

    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self.key_count = Some(1);
        self
    }

    pub fn with_release(mut self, title: impl Into<String>, emergency: bool) -> Self {
        self.release_title = Some(title.into());
        self.emergency = Some(emergency);
        self
    }

    pub fn with_release_ids(mut self, release_id: i64, to_release_id: Option<i64>) -> Self {
        self.release_id = Some(release_id);
        self.to_release_id = to_release_id;
        self
    }

    pub fn with_namespace_kind(mut self, public: bool, append_prefix: bool) -> Self {
        self.public_namespace = Some(public);
        self.append_namespace_prefix = Some(append_prefix);
        self
    }

    pub fn with_request(mut self, method: impl Into<String>, path: &str) -> Self {
        self.request = Some(MutationRequest::new(method, path));
        self
    }

    pub fn render_table(&self) -> String {
        let mut lines = vec![
            "Mutation plan:".to_owned(),
            format!("Operation: {}", table_value(&self.operation)),
        ];
        push_optional(&mut lines, "Profile", self.profile.as_deref());
        push_optional(&mut lines, "Server", self.server.as_deref());
        if let Some(source) = &self.source {
            lines.push(format!("Source: {}", source.render_table()));
        }
        if let Some(target) = &self.target {
            lines.push(format!("Target: {}", target.render_table()));
        }
        push_optional(&mut lines, "Key", self.key.as_deref());
        if let Some(key_count) = self.key_count {
            lines.push(format!("Key count: {key_count}"));
        }
        push_optional(&mut lines, "Release title", self.release_title.as_deref());
        if let Some(emergency) = self.emergency {
            lines.push(format!("Emergency: {emergency}"));
        }
        if let Some(release_id) = self.release_id {
            lines.push(format!("Release ID: {release_id}"));
        }
        if let Some(to_release_id) = self.to_release_id {
            lines.push(format!("Target release ID: {to_release_id}"));
        }
        if let Some(public_namespace) = self.public_namespace {
            lines.push(format!("Public namespace: {public_namespace}"));
        }
        if let Some(append_namespace_prefix) = self.append_namespace_prefix {
            lines.push(format!(
                "Append namespace prefix: {append_namespace_prefix}"
            ));
        }
        if let Some(request) = &self.request {
            lines.push(format!("Method: {}", table_value(&request.method)));
            lines.push(format!("Path: {}", table_value(&request.path)));
            if !request.query_parameters.is_empty() {
                lines.push(format!(
                    "Query parameters: {}",
                    request
                        .query_parameters
                        .iter()
                        .map(|value| table_value(value))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        lines.join("\n")
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationScope {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

impl MutationScope {
    pub fn namespace(
        app: impl Into<String>,
        env: impl Into<String>,
        cluster: impl Into<String>,
        namespace: impl Into<String>,
    ) -> Self {
        Self {
            app: Some(app.into()),
            env: Some(env.into()),
            cluster: Some(cluster.into()),
            namespace: Some(namespace.into()),
        }
    }

    pub fn environment(env: impl Into<String>) -> Self {
        Self {
            app: None,
            env: Some(env.into()),
            cluster: None,
            namespace: None,
        }
    }

    fn render_table(&self) -> String {
        let fields = [
            ("app", self.app.as_deref()),
            ("env", self.env.as_deref()),
            ("cluster", self.cluster.as_deref()),
            ("namespace", self.namespace.as_deref()),
        ];
        fields
            .into_iter()
            .filter_map(|(name, value)| value.map(|value| format!("{name}={}", table_value(value))))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationRequest {
    pub method: String,
    pub path: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub query_parameters: Vec<String>,
}

impl MutationRequest {
    fn new(method: impl Into<String>, path: &str) -> Self {
        let path = path.split('#').next().unwrap_or_default();
        let (path, query) = path.split_once('?').unwrap_or((path, ""));
        let query_parameters = query
            .split('&')
            .filter_map(|pair| pair.split('=').next())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        Self {
            method: method.into(),
            path: sanitize_path(path),
            query_parameters,
        }
    }
}

fn sanitize_server(server: &str) -> String {
    let without_fragment = server.split('#').next().unwrap_or_default();
    let without_query = without_fragment.split('?').next().unwrap_or_default();
    let Some((scheme, remainder)) = without_query.split_once("://") else {
        return without_query.to_owned();
    };
    let authority_end = remainder.find('/').unwrap_or(remainder.len());
    let (authority, suffix) = remainder.split_at(authority_end);
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    format!("{scheme}://{authority}{}", sanitize_path(suffix))
}

fn sanitize_path(path: &str) -> String {
    let mut redact_next = false;
    path.split('/')
        .map(|segment| {
            if redact_next {
                redact_next = false;
                return "[REDACTED]".to_owned();
            }
            let lowercase = segment.to_ascii_lowercase();
            redact_next = lowercase.contains("token")
                || lowercase.contains("authorization")
                || lowercase.contains("password")
                || lowercase.contains("secret");
            segment.to_owned()
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn push_optional(lines: &mut Vec<String>, label: &str, value: Option<&str>) {
    if let Some(value) = value {
        lines.push(format!("{label}: {}", table_value(value)));
    }
}

fn table_value(value: &str) -> String {
    let mut truncated = false;
    let value = value
        .chars()
        .filter_map(|character| {
            if character == '\n' || character == '\r' || character == '\t' {
                Some(' ')
            } else if character.is_control() {
                None
            } else {
                Some(character)
            }
        })
        .take(MAX_TABLE_VALUE_CHARS + 1)
        .enumerate()
        .filter_map(|(index, character)| {
            if index == MAX_TABLE_VALUE_CHARS {
                truncated = true;
                None
            } else {
                Some(character)
            }
        })
        .collect::<String>();
    if truncated {
        format!("{value}...")
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::{MutationPlan, MutationScope};

    #[test]
    fn request_plan_omits_query_values_and_sensitive_path_segments() {
        let plan = MutationPlan::new("api.post")
            .with_context(
                Some("dev".to_owned()),
                Some("https://user:password@example.com/tokens/server-secret?token=secret"),
            )
            .with_request(
                "POST",
                "/openapi/v1/tokens/consumer-secret/apps?operator=alice&token=secret",
            );
        let json = serde_json::to_value(&plan).expect("plan json");

        assert_eq!(json["server"], "https://example.com/tokens/[REDACTED]");
        assert_eq!(
            json["request"]["path"],
            "/openapi/v1/tokens/[REDACTED]/apps"
        );
        assert_eq!(json["request"]["queryParameters"][0], "operator");
        assert_eq!(json["request"]["queryParameters"][1], "token");
        assert!(!json.to_string().contains("consumer-secret"));
    }

    #[test]
    fn table_plan_replaces_control_characters() {
        let plan = MutationPlan::new("config.set")
            .with_target(MutationScope::namespace(
                "demo",
                "PROD",
                "default",
                "application",
            ))
            .with_key("safe\nInjected: value");

        let table = plan.render_table();
        assert!(table.contains("Key: safe Injected: value"));
        assert!(!table.contains("safe\nInjected"));
    }
}
