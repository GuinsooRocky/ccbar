use std::process::Command;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProviderActivity {
    pub claude: bool,
    pub codex: bool,
}

pub fn detect() -> ProviderActivity {
    let output = match Command::new("/bin/ps")
        .args(["-axo", "pid=,command="])
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return ProviderActivity::default(),
    };
    detect_from_process_list(&String::from_utf8_lossy(&output.stdout))
}

fn detect_from_process_list(processes: &str) -> ProviderActivity {
    let mut activity = ProviderActivity::default();

    for line in processes.lines() {
        let command = line
            .trim_start()
            .split_once(char::is_whitespace)
            .map(|(_, command)| command.trim_start())
            .unwrap_or("");
        let lower = command.to_ascii_lowercase();

        if lower.contains("/applications/claude.app/") {
            activity.claude = true;
        } else if executable_name(&lower) == "claude"
            && !lower.contains(" mcp serve")
            && !lower.contains(" --chrome-native-host")
        {
            activity.claude = true;
        }

        if lower.contains("/applications/codex.app/") {
            activity.codex = true;
        } else if executable_name(&lower) == "codex" && !lower.contains(" mcp-server") {
            activity.codex = true;
        }

        if activity.claude && activity.codex {
            break;
        }
    }

    activity
}

fn executable_name(command: &str) -> &str {
    command
        .split_ascii_whitespace()
        .next()
        .unwrap_or("")
        .rsplit('/')
        .next()
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_apps_and_interactive_clis() {
        let processes = r#"
          101 /Applications/Codex.app/Contents/MacOS/Codex
          102 claude --dangerously-skip-permissions
        "#;
        assert_eq!(
            detect_from_process_list(processes),
            ProviderActivity {
                claude: true,
                codex: true,
            }
        );
    }

    #[test]
    fn ignores_background_mcp_servers() {
        let processes = r#"
          201 claude mcp serve
          202 codex mcp-server
          203 /bin/zsh -c echo codex
        "#;
        assert_eq!(
            detect_from_process_list(processes),
            ProviderActivity::default()
        );
    }

    #[test]
    fn detects_app_helpers_while_desktop_app_is_open() {
        let processes = r#"
          301 /Applications/Claude.app/Contents/Frameworks/Claude Helper.app/Contents/MacOS/Claude Helper
          302 /Applications/Codex.app/Contents/Frameworks/Codex Framework.framework/Helpers/helper
        "#;
        let activity = detect_from_process_list(processes);
        assert!(activity.claude);
        assert!(activity.codex);
    }
}
