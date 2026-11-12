use serde_json::Value;
use t3_client::projection::ShellState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub environment: String,
    pub project_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectGroup {
    pub key: String,
    pub title: String,
    pub members: Vec<Member>,
    pub updated_at: String,
}

impl ProjectGroup {
    pub fn target(&self) -> &Member {
        &self.members[0]
    }

    pub fn contains(&self, environment: &str, project_id: &str) -> bool {
        self.members
            .iter()
            .any(|m| m.environment == environment && m.project_id == project_id)
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn group_key(environment: &str, project: &Value) -> String {
    let identity = project.get("repositoryIdentity").unwrap_or(&Value::Null);
    let repository = identity
        .pointer("/origin/canonicalKey")
        .and_then(Value::as_str)
        .or_else(|| identity.get("canonicalKey").and_then(Value::as_str))
        .filter(|key| !key.is_empty() && !key.contains('\0'));
    match repository {
        Some(key) => key.to_string(),
        None => format!(
            "{environment}:{}",
            text(project, "workspaceRoot").trim_end_matches('/')
        ),
    }
}

fn latest_thread<'a>(shell: &'a ShellState, project_id: &str) -> Option<&'a Value> {
    shell
        .threads
        .iter()
        .filter(|t| text(t, "projectId") == project_id)
        .max_by(|a, b| text(a, "updatedAt").cmp(text(b, "updatedAt")))
}

/// Projects of every host, one group per repository, in host order inside a group so the first
/// member is on the primary host. Groups are ordered by their latest thread.
pub fn groups<'a>(
    shells: impl IntoIterator<Item = (&'a str, &'a ShellState)>,
) -> Vec<ProjectGroup> {
    let mut groups: Vec<ProjectGroup> = Vec::new();
    for (environment, shell) in shells {
        for project in &shell.projects {
            if project.get("archivedAt").is_some_and(|v| !v.is_null()) {
                continue;
            }
            let project_id = text(project, "id");
            let updated_at = latest_thread(shell, project_id)
                .map(|t| text(t, "updatedAt"))
                .unwrap_or(text(project, "updatedAt"))
                .to_string();
            let member = Member {
                environment: environment.to_string(),
                project_id: project_id.to_string(),
            };
            let key = group_key(environment, project);
            match groups.iter_mut().find(|g| g.key == key) {
                Some(group) => {
                    group.members.push(member);
                    if updated_at > group.updated_at {
                        group.updated_at = updated_at;
                    }
                }
                None => groups.push(ProjectGroup {
                    key,
                    title: text(project, "title").to_string(),
                    members: vec![member],
                    updated_at,
                }),
            }
        }
    }
    groups.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    groups
}

pub fn title_seed(message: &str) -> String {
    let line = message
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .trim();
    line.chars().take(80).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn shell(projects: Vec<Value>, threads: Vec<Value>) -> ShellState {
        ShellState {
            sequence: 1,
            projects,
            threads,
            synchronized: true,
        }
    }

    fn repo(id: &str, key: &str) -> Value {
        json!({"id": id, "title": "idea", "workspaceRoot": format!("/home/{id}"),
               "repositoryIdentity": {"canonicalKey": key}, "defaultModelSelection": null,
               "updatedAt": "2026-11-01T00:00:00.000Z"})
    }

    #[test]
    fn one_repository_on_two_hosts_is_one_group_led_by_the_primary_host() {
        let primary = shell(
            vec![
                repo("p-eos", "github.com/conao3/idea"),
                json!({"id": "scratch", "title": "tmp", "workspaceRoot": "/tmp/x/"}),
            ],
            vec![],
        );
        let remote = shell(
            vec![
                repo("p-helios", "github.com/conao3/idea"),
                json!({"id": "gone", "title": "old", "workspaceRoot": "/o", "archivedAt": "2026-01-01T00:00:00.000Z"}),
            ],
            vec![
                json!({"id": "t", "projectId": "p-helios", "updatedAt": "2026-11-12T00:00:00.000Z"}),
            ],
        );
        let groups = groups([("eos", &primary), ("helios", &remote)]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].key, "github.com/conao3/idea");
        assert_eq!(groups[0].updated_at, "2026-11-12T00:00:00.000Z");
        assert_eq!(groups[0].target().environment, "eos");
        assert_eq!(groups[0].target().project_id, "p-eos");
        assert!(groups[0].contains("helios", "p-helios"));
        assert_eq!(groups[1].key, "eos:/tmp/x");
    }

    #[test]
    fn the_title_seed_is_the_first_line() {
        assert_eq!(title_seed("\n  Fix the build  \nmore"), "Fix the build");
        assert_eq!(title_seed(&"あ".repeat(100)).chars().count(), 80);
    }
}
