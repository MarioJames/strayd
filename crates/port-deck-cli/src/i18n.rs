use std::collections::BTreeMap;
use std::sync::OnceLock;

use port_deck_engine::{EngineAction, EngineError};
use serde::Deserialize;

use crate::{ConfigError, PlanError};

const EN_US_SOURCE: &str = include_str!("../locales/en-US.toml");
const ZH_CN_SOURCE: &str = include_str!("../locales/zh-CN.toml");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    ZhCn,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocaleResource {
    messages: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy)]
pub struct Translator {
    language: Language,
    messages: &'static BTreeMap<String, String>,
}

impl Translator {
    pub fn new(language: Language) -> Self {
        Self {
            language,
            messages: locale_messages(language),
        }
    }

    pub const fn language(self) -> Language {
        self.language
    }

    pub fn text(self, key: &str) -> &'static str {
        self.messages
            .get(key)
            .map(String::as_str)
            .unwrap_or_else(|| panic!("missing locale key: {key}"))
    }

    pub fn format(self, key: &str, values: &[(&str, String)]) -> String {
        values
            .iter()
            .fold(self.text(key).to_owned(), |text, (key, value)| {
                text.replace(&format!("{{{key}}}"), value)
            })
    }

    pub fn no_matching_resources(self) -> &'static str {
        self.text("no_matching_resources")
    }

    pub fn stopped(self, count: usize) -> String {
        self.format("stopped", &[("count", count.to_string())])
    }

    pub fn scan_status(
        self,
        groups: usize,
        resources: usize,
        warnings: usize,
        hidden: usize,
    ) -> String {
        let hidden = if hidden > 0 {
            self.format("tui_hidden_status", &[("count", hidden.to_string())])
        } else {
            String::new()
        };
        let key = if warnings == 0 {
            "tui_scan_ready"
        } else {
            "tui_scan_warnings"
        };
        self.format(
            key,
            &[
                ("groups", groups.to_string()),
                ("resources", resources.to_string()),
                ("warnings", warnings.to_string()),
                ("hidden", hidden),
            ],
        )
    }

    pub fn plan_error(self, error: &PlanError) -> String {
        match error {
            PlanError::NoMatches => self.text("plan_no_matches").into(),
            PlanError::MultipleMatches(count) => {
                self.format("plan_multiple_matches", &[("count", count.to_string())])
            }
            PlanError::ProtectedGroup(group) => {
                self.format("plan_protected_group", &[("group", group.clone())])
            }
        }
    }

    pub fn config_error(self, error: &ConfigError) -> String {
        match error {
            ConfigError::DirectoryUnavailable => self.text("config_directory_unavailable").into(),
            ConfigError::Read { path, source } => self.format(
                "config_read_error",
                &[
                    ("path", path.display().to_string()),
                    ("error", source.to_string()),
                ],
            ),
            ConfigError::Parse { path, message } => self.format(
                "config_parse_error",
                &[
                    ("path", path.display().to_string()),
                    ("error", message.clone()),
                ],
            ),
            ConfigError::UnsupportedVersion(version) => self.format(
                "config_unsupported_version",
                &[("version", version.to_string())],
            ),
            ConfigError::AlreadyExists(path) => self.format(
                "config_already_exists",
                &[("path", path.display().to_string())],
            ),
            ConfigError::Write { path, source } => self.format(
                "config_write_error",
                &[
                    ("path", path.display().to_string()),
                    ("error", source.to_string()),
                ],
            ),
        }
    }

    pub fn engine_error(self, error: &EngineError) -> String {
        match error {
            EngineError::WrongPlatform => self.text("engine_wrong_platform").into(),
            EngineError::ProtectedProcess => self.text("engine_protected_process").into(),
            EngineError::MissingStartToken => self.text("engine_missing_start_token").into(),
            EngineError::InvalidServiceUnit => self.text("engine_invalid_service_unit").into(),
            EngineError::OpenUrl { url, detail } => self.format(
                "engine_open_url",
                &[("url", url.clone()), ("error", detail.clone())],
            ),
            EngineError::Spawn { program, detail } => self.format(
                "engine_spawn",
                &[("program", program.clone()), ("error", detail.clone())],
            ),
            EngineError::SignalDenied => self.text("engine_signal_denied").into(),
            EngineError::ProcessMissing => self.text("engine_process_missing").into(),
            EngineError::ProcessChanged => self.text("engine_process_changed").into(),
            EngineError::ProtectedSshd => self.text("engine_protected_sshd").into(),
            EngineError::ManagedRelationChanged => {
                self.text("engine_managed_relation_changed").into()
            }
            EngineError::CommandFailed {
                action,
                exit_code,
                detail,
            } => {
                let action = match action {
                    EngineAction::StopWindowsProcess => self.text("engine_stop_windows"),
                    EngineAction::StopSystemdService => self.text("engine_stop_systemd"),
                };
                if let Some(detail) = detail {
                    self.format(
                        "engine_command_failed_detail",
                        &[("action", action.into()), ("detail", detail.clone())],
                    )
                } else {
                    self.format(
                        "engine_command_failed_code",
                        &[
                            ("action", action.into()),
                            (
                                "code",
                                exit_code.map_or_else(|| "?".into(), |code| code.to_string()),
                            ),
                        ],
                    )
                }
            }
        }
    }
}

fn locale_messages(language: Language) -> &'static BTreeMap<String, String> {
    static EN_US: OnceLock<LocaleResource> = OnceLock::new();
    static ZH_CN: OnceLock<LocaleResource> = OnceLock::new();
    match language {
        Language::English => {
            &EN_US
                .get_or_init(|| parse_locale(EN_US_SOURCE, "en-US"))
                .messages
        }
        Language::ZhCn => {
            &ZH_CN
                .get_or_init(|| parse_locale(ZH_CN_SOURCE, "zh-CN"))
                .messages
        }
    }
}

fn parse_locale(source: &str, name: &str) -> LocaleResource {
    toml::from_str(source).unwrap_or_else(|error| panic!("invalid {name} locale resource: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{EN_US_SOURCE, ZH_CN_SOURCE, parse_locale};

    #[test]
    fn locale_resources_have_identical_keys_and_english_contains_no_cjk_copy() {
        let english = parse_locale(EN_US_SOURCE, "en-US");
        let chinese = parse_locale(ZH_CN_SOURCE, "zh-CN");

        assert_eq!(
            english.messages.keys().collect::<Vec<_>>(),
            chinese.messages.keys().collect::<Vec<_>>()
        );
        assert!(english.messages.values().all(|value| {
            !value
                .chars()
                .any(|character| ('\u{4e00}'..='\u{9fff}').contains(&character))
        }));
    }
}
