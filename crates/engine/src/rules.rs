use crate::{Library, RuleAction};
use rbw_platform::{Architecture, Platform};
use regex::Regex;

#[derive(Debug, Clone)]
pub struct RuleContext<'a> {
    pub platform: Platform,
    pub os_version: Option<&'a str>,
}

pub fn library_is_allowed(library: &Library, context: &RuleContext<'_>) -> bool {
    if library.rules.is_empty() {
        return true;
    }

    let mut allowed = false;
    for rule in &library.rules {
        if rule_matches(rule.os.as_ref(), context) {
            allowed = rule.action == RuleAction::Allow;
        }
    }
    allowed
}

pub fn native_classifier(library: &Library, platform: Platform) -> Option<String> {
    library
        .natives
        .get(platform.os.minecraft_rule_name())
        .map(|classifier| classifier.replace("${arch}", platform.game_arch.bits()))
}

fn rule_matches(os_rule: Option<&crate::RuleOs>, context: &RuleContext<'_>) -> bool {
    let Some(os_rule) = os_rule else {
        return true;
    };

    if let Some(name) = &os_rule.name
        && name != context.platform.os.minecraft_rule_name()
    {
        return false;
    }

    if let Some(architecture) = &os_rule.arch
        && !architecture_matches(architecture, context.platform.game_arch)
    {
        return false;
    }

    if let Some(version_pattern) = &os_rule.version {
        let Some(version) = context.os_version else {
            return false;
        };
        let Ok(pattern) = Regex::new(version_pattern) else {
            return false;
        };
        if !pattern.is_match(version) {
            return false;
        }
    }

    true
}

fn architecture_matches(rule: &str, architecture: Architecture) -> bool {
    match rule.to_ascii_lowercase().as_str() {
        "x86" | "i386" | "i686" => architecture == Architecture::X86,
        "x86_64" | "amd64" => architecture == Architecture::X86_64,
        "arm64" | "aarch64" => architecture == Architecture::Aarch64,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LibraryDownloads, Rule, RuleOs};
    use rbw_platform::OperatingSystem;
    use std::collections::BTreeMap;

    fn library(rules: Vec<Rule>) -> Library {
        Library {
            name: "test:library:1".to_owned(),
            downloads: LibraryDownloads {
                artifact: None,
                classifiers: BTreeMap::new(),
            },
            rules,
            natives: BTreeMap::new(),
            extract: None,
        }
    }

    fn mac_arm_host_x64_game() -> RuleContext<'static> {
        RuleContext {
            platform: Platform {
                os: OperatingSystem::MacOs,
                host_arch: Architecture::Aarch64,
                game_arch: Architecture::X86_64,
            },
            os_version: Some("26.6"),
        }
    }

    #[test]
    fn no_rules_means_allowed() {
        assert!(library_is_allowed(
            &library(vec![]),
            &mac_arm_host_x64_game()
        ));
    }

    #[test]
    fn last_matching_rule_wins() {
        let rules = vec![
            Rule {
                action: RuleAction::Allow,
                os: None,
            },
            Rule {
                action: RuleAction::Disallow,
                os: Some(RuleOs {
                    name: Some("osx".to_owned()),
                    arch: None,
                    version: None,
                }),
            },
        ];
        assert!(!library_is_allowed(
            &library(rules),
            &mac_arm_host_x64_game()
        ));
    }

    #[test]
    fn native_classifier_uses_game_architecture() {
        let mut candidate = library(vec![]);
        candidate
            .natives
            .insert("osx".to_owned(), "natives-osx-${arch}".to_owned());
        assert_eq!(
            native_classifier(&candidate, mac_arm_host_x64_game().platform).as_deref(),
            Some("natives-osx-64")
        );
    }
}
