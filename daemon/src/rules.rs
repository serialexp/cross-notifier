// Rule matching engine for notification filtering.
// Evaluates notification rules in order; first match wins.
// Supports filtering by server label, source, status, and regex pattern on title+message.

use regex::Regex;

use crate::config::{NotificationRule, RuleAction, RulesConfig};
use crate::notification::NotificationPayload;

/// Result of matching a notification against the rule set.
pub struct RuleMatch<'a> {
    pub sound: &'a str,
    pub action: &'a RuleAction,
}

/// Find the first matching rule for a notification.
/// Returns None if rules are disabled or no rule matches.
pub fn match_rule<'a>(
    payload: &NotificationPayload,
    server_label: &str,
    rules: &'a RulesConfig,
) -> Option<RuleMatch<'a>> {
    if !rules.enabled {
        return None;
    }

    for rule in &rules.rules {
        if matches_rule(payload, server_label, rule) {
            return Some(RuleMatch {
                sound: &rule.sound,
                action: rule.effective_action(),
            });
        }
    }

    None
}

fn matches_rule(
    payload: &NotificationPayload,
    server_label: &str,
    rule: &NotificationRule,
) -> bool {
    // Server filter: empty = any
    if !rule.server.is_empty() && rule.server != server_label {
        return false;
    }

    // Source filter: empty = any
    if !rule.source.is_empty() && rule.source != payload.source {
        return false;
    }

    // Status filter: empty = any
    if !rule.status.is_empty() && rule.status != payload.status {
        return false;
    }

    // Pattern filter: empty = any, regex match on "title message"
    if !rule.pattern.is_empty() {
        let text = format!("{} {}", payload.title, payload.message);
        match Regex::new(&rule.pattern) {
            Ok(re) => {
                if !re.is_match(&text) {
                    return false;
                }
            }
            Err(_) => {
                // Invalid regex — skip this rule
                return false;
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NotificationRule;

    fn make_payload(title: &str, message: &str, source: &str, status: &str) -> NotificationPayload {
        NotificationPayload {
            title: title.to_string(),
            message: message.to_string(),
            source: source.to_string(),
            status: status.to_string(),
            ..default_payload()
        }
    }

    fn default_payload() -> NotificationPayload {
        NotificationPayload {
            id: String::new(),
            source: String::new(),
            title: String::new(),
            message: String::new(),
            status: String::new(),
            icon_data: String::new(),
            icon_href: String::new(),
            icon_path: String::new(),
            duration: 0,
            actions: vec![],
            exclusive: false,
            store_on_expire: true,
        }
    }

    fn make_rule(server: &str, source: &str, status: &str, pattern: &str, sound: &str, action: RuleAction) -> NotificationRule {
        NotificationRule {
            server: server.to_string(),
            source: source.to_string(),
            status: status.to_string(),
            pattern: pattern.to_string(),
            sound: sound.to_string(),
            action,
            suppress: false,
        }
    }

    #[test]
    fn test_disabled_rules_return_none() {
        let rules = RulesConfig {
            enabled: false,
            rules: vec![make_rule("", "", "", "", "Bell", RuleAction::Normal)],
        };
        let payload = make_payload("Test", "Hello", "", "info");
        assert!(match_rule(&payload, "Work", &rules).is_none());
    }

    #[test]
    fn test_empty_rules_return_none() {
        let rules = RulesConfig {
            enabled: true,
            rules: vec![],
        };
        let payload = make_payload("Test", "Hello", "", "info");
        assert!(match_rule(&payload, "Work", &rules).is_none());
    }

    #[test]
    fn test_wildcard_rule_matches_all() {
        let rules = RulesConfig {
            enabled: true,
            rules: vec![make_rule("", "", "", "", "Bell", RuleAction::Normal)],
        };
        let payload = make_payload("Test", "Hello", "github", "info");
        let m = match_rule(&payload, "Work", &rules).unwrap();
        assert_eq!(m.sound, "Bell");
        assert_eq!(m.action, &RuleAction::Normal);
    }

    #[test]
    fn test_server_filter() {
        let rules = RulesConfig {
            enabled: true,
            rules: vec![make_rule("Work", "", "", "", "Bell", RuleAction::Normal)],
        };
        let payload = make_payload("Test", "Hello", "", "info");

        // Matches
        assert!(match_rule(&payload, "Work", &rules).is_some());
        // Doesn't match
        assert!(match_rule(&payload, "Home", &rules).is_none());
    }

    #[test]
    fn test_source_filter() {
        let rules = RulesConfig {
            enabled: true,
            rules: vec![make_rule("", "github", "", "", "Bell", RuleAction::Normal)],
        };
        assert!(match_rule(&make_payload("Test", "Hello", "github", ""), "Work", &rules).is_some());
        assert!(match_rule(&make_payload("Test", "Hello", "gitlab", ""), "Work", &rules).is_none());
    }

    #[test]
    fn test_status_filter() {
        let rules = RulesConfig {
            enabled: true,
            rules: vec![make_rule("", "", "error", "", "Bell", RuleAction::Normal)],
        };
        assert!(match_rule(&make_payload("Test", "Hello", "", "error"), "Work", &rules).is_some());
        assert!(match_rule(&make_payload("Test", "Hello", "", "info"), "Work", &rules).is_none());
    }

    #[test]
    fn test_pattern_filter() {
        let rules = RulesConfig {
            enabled: true,
            rules: vec![make_rule("", "", "", "(?i)urgent", "Bell", RuleAction::Normal)],
        };
        // Matches in title
        assert!(match_rule(&make_payload("URGENT: fix", "details", "", ""), "Work", &rules).is_some());
        // Matches in message
        assert!(match_rule(&make_payload("Build", "urgent fix needed", "", ""), "Work", &rules).is_some());
        // Doesn't match
        assert!(match_rule(&make_payload("Build", "completed", "", ""), "Work", &rules).is_none());
    }

    #[test]
    fn test_invalid_regex_skips_rule() {
        let rules = RulesConfig {
            enabled: true,
            rules: vec![
                make_rule("", "", "", "[invalid", "Bell", RuleAction::Normal),
                make_rule("", "", "", "", "Pop", RuleAction::Normal),
            ],
        };
        let payload = make_payload("Test", "Hello", "", "");
        let m = match_rule(&payload, "Work", &rules).unwrap();
        // Should skip the invalid regex rule and match the wildcard
        assert_eq!(m.sound, "Pop");
    }

    #[test]
    fn test_combined_filters() {
        let rules = RulesConfig {
            enabled: true,
            rules: vec![make_rule("Work", "github", "error", "(?i)deploy", "Bell", RuleAction::Normal)],
        };
        // All match
        assert!(match_rule(
            &make_payload("Deploy failed", "Check logs", "github", "error"),
            "Work",
            &rules
        ).is_some());
        // Wrong server
        assert!(match_rule(
            &make_payload("Deploy failed", "Check logs", "github", "error"),
            "Home",
            &rules
        ).is_none());
        // Wrong source
        assert!(match_rule(
            &make_payload("Deploy failed", "Check logs", "gitlab", "error"),
            "Work",
            &rules
        ).is_none());
        // Wrong status
        assert!(match_rule(
            &make_payload("Deploy failed", "Check logs", "github", "info"),
            "Work",
            &rules
        ).is_none());
        // Wrong pattern
        assert!(match_rule(
            &make_payload("Build completed", "All good", "github", "error"),
            "Work",
            &rules
        ).is_none());
    }

    #[test]
    fn test_first_match_wins() {
        let rules = RulesConfig {
            enabled: true,
            rules: vec![
                make_rule("", "", "error", "", "Bell", RuleAction::Dismiss),
                make_rule("", "", "", "", "Pop", RuleAction::Normal),
            ],
        };
        // Error notification matches first rule
        let m = match_rule(&make_payload("Test", "Hello", "", "error"), "Work", &rules).unwrap();
        assert_eq!(m.sound, "Bell");
        assert_eq!(m.action, &RuleAction::Dismiss);

        // Info notification matches second rule
        let m = match_rule(&make_payload("Test", "Hello", "", "info"), "Work", &rules).unwrap();
        assert_eq!(m.sound, "Pop");
        assert_eq!(m.action, &RuleAction::Normal);
    }

    #[test]
    fn test_actions() {
        let rules = RulesConfig {
            enabled: true,
            rules: vec![
                make_rule("", "", "", "", "", RuleAction::Silent),
            ],
        };
        let m = match_rule(&make_payload("Test", "Hello", "", ""), "Work", &rules).unwrap();
        assert_eq!(m.action, &RuleAction::Silent);
    }

    #[test]
    fn test_suppress_backward_compat() {
        let rules = RulesConfig {
            enabled: true,
            rules: vec![NotificationRule {
                server: String::new(),
                source: String::new(),
                status: String::new(),
                pattern: String::new(),
                sound: String::new(),
                action: RuleAction::Normal,
                suppress: true,
            }],
        };
        let m = match_rule(&make_payload("Test", "Hello", "", ""), "Work", &rules).unwrap();
        assert_eq!(m.action, &RuleAction::Dismiss);
    }
}
