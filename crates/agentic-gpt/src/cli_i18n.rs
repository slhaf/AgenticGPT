use std::ffi::{OsStr, OsString};

use crate::config_templates::PendingAction;

#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub(crate) enum LanguageChoice {
    Auto,
    #[value(name = "zh-CN", alias = "zh", alias = "zh_CN")]
    ZhCn,
    #[value(name = "en", alias = "en-US", alias = "en_US")]
    En,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UiLanguage {
    ZhCn,
    En,
}

pub(crate) trait LocaleSource {
    fn get(&self, key: &str) -> Option<OsString>;
}

pub(crate) fn resolve_language(choice: LanguageChoice, env: &impl LocaleSource) -> UiLanguage {
    match choice {
        LanguageChoice::ZhCn => UiLanguage::ZhCn,
        LanguageChoice::En => UiLanguage::En,
        LanguageChoice::Auto => {
            for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
                if let Some(locale) = env.get(key) {
                    return normalize_locale(&locale);
                }
            }
            UiLanguage::En
        }
    }
}

pub(crate) fn process_language() -> UiLanguage {
    let args = std::env::args_os().collect::<Vec<_>>();
    let choice = prescan_language(&args).unwrap_or(LanguageChoice::Auto);
    resolve_language(choice, &ProcessLocale)
}

struct ProcessLocale;

impl LocaleSource for ProcessLocale {
    fn get(&self, key: &str) -> Option<OsString> {
        std::env::var_os(key)
    }
}

pub(crate) fn prescan_language(args: &[OsString]) -> Result<LanguageChoice, String> {
    let mut choice = LanguageChoice::Auto;
    let mut index = 0;

    while index < args.len() {
        let argument = args[index].to_string_lossy();
        if let Some(value) = argument.strip_prefix("--language=") {
            choice = parse_language_value(value)?;
        } else if argument == "--language" {
            let value = args
                .get(index + 1)
                .ok_or_else(|| "--language requires a value".to_string())?;
            let value = value
                .to_str()
                .ok_or_else(|| "--language value must be valid UTF-8".to_string())?;
            choice = parse_language_value(value)?;
            index += 1;
        }
        index += 1;
    }

    Ok(choice)
}

fn parse_language_value(value: &str) -> Result<LanguageChoice, String> {
    match value {
        "auto" => Ok(LanguageChoice::Auto),
        "zh-CN" | "zh" | "zh_CN" => Ok(LanguageChoice::ZhCn),
        "en" | "en-US" | "en_US" => Ok(LanguageChoice::En),
        _ => Err(format!("invalid value '{value}' for --language")),
    }
}

fn normalize_locale(value: &OsStr) -> UiLanguage {
    let Some(value) = value.to_str() else {
        return UiLanguage::En;
    };
    let normalized = value.to_ascii_lowercase().replace('_', "-");
    if normalized.starts_with("zh") {
        UiLanguage::ZhCn
    } else {
        UiLanguage::En
    }
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Remaining CLI catalog entries are reserved for later command help and init flows."
    )
)]
pub(crate) struct CliText {
    pub app_about: &'static str,
    pub config_about: &'static str,
    pub config_init_about: &'static str,
    pub config_keys_about: &'static str,
    pub config_set_about: &'static str,
    pub usage_heading: &'static str,
    pub commands_heading: &'static str,
    pub options_heading: &'static str,
    pub arguments_heading: &'static str,
    pub help_flag: &'static str,
    pub version_flag: &'static str,
    pub cancelled: &'static str,
    pub initialized: &'static str,
    pub replace_tunnel_id: &'static str,
    pub provision_tunnel_secret: &'static str,
    pub configure_hub_url: &'static str,
    pub replace_agent_secret: &'static str,
    pub optional_settings_prompt: &'static str,
    pub safe_defaults_option: &'static str,
    pub choose_sections_option: &'static str,
}

pub(crate) static ZH_CN_TEXT: CliText = CliText {
    app_about: "Agentic GPT Linux 本地代理",
    config_about: "管理 Agentic GPT 配置",
    config_init_about: "初始化配置文件",
    config_keys_about: "列出配置键",
    config_set_about: "设置配置值",
    usage_heading: "用法",
    commands_heading: "命令",
    options_heading: "选项",
    arguments_heading: "参数",
    help_flag: "显示帮助信息",
    version_flag: "显示版本信息",
    cancelled: "已取消。",
    initialized: "配置已初始化。",
    replace_tunnel_id: "下一步：替换 tunnel ID。",
    provision_tunnel_secret: "下一步：配置 tunnel API 密钥引用。",
    configure_hub_url: "下一步：配置 Hub URL。",
    replace_agent_secret: "下一步：替换代理密钥。",
    optional_settings_prompt: "是否配置可选设置？",
    safe_defaults_option: "使用安全默认值",
    choose_sections_option: "选择要配置的部分",
};

pub(crate) static EN_TEXT: CliText = CliText {
    app_about: "Linux local agent for Agentic GPT",
    config_about: "Manage Agentic GPT configuration",
    config_init_about: "Initialize the configuration file",
    config_keys_about: "List configuration keys",
    config_set_about: "Set a configuration value",
    usage_heading: "Usage",
    commands_heading: "Commands",
    options_heading: "Options",
    arguments_heading: "Arguments",
    help_flag: "Print help information",
    version_flag: "Print version information",
    cancelled: "Cancelled.",
    initialized: "Configuration initialized.",
    replace_tunnel_id: "Next step: replace tunnel ID.",
    provision_tunnel_secret: "Next step: provision the tunnel API secret reference.",
    configure_hub_url: "Next step: configure the Hub URL.",
    replace_agent_secret: "Next step: replace the agent secret.",
    optional_settings_prompt: "Configure optional settings?",
    safe_defaults_option: "Use safe defaults",
    choose_sections_option: "Choose sections to configure",
};

pub(crate) fn text(language: UiLanguage) -> &'static CliText {
    match language {
        UiLanguage::ZhCn => &ZH_CN_TEXT,
        UiLanguage::En => &EN_TEXT,
    }
}

pub(crate) fn pending_action_text(action: PendingAction, language: UiLanguage) -> &'static str {
    let catalog = text(language);
    match action {
        PendingAction::ReplaceTunnelId => catalog.replace_tunnel_id,
        PendingAction::ProvisionTunnelSecret => catalog.provision_tunnel_secret,
        PendingAction::ConfigureHubUrl => catalog.configure_hub_url,
        PendingAction::ReplaceAgentSecret => catalog.replace_agent_secret,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, ffi::OsString};

    #[derive(Default)]
    struct TestEnv(BTreeMap<String, OsString>);

    impl LocaleSource for TestEnv {
        fn get(&self, key: &str) -> Option<OsString> {
            self.0.get(key).cloned()
        }
    }

    #[test]
    fn explicit_language_overrides_locale_environment() {
        let env = TestEnv(BTreeMap::from([("LC_ALL".into(), "en_US.UTF-8".into())]));
        assert_eq!(
            resolve_language(LanguageChoice::ZhCn, &env),
            UiLanguage::ZhCn
        );
    }

    #[test]
    fn locale_precedence_is_lc_all_then_lc_messages_then_lang() {
        let env = TestEnv(BTreeMap::from([
            ("LANG".into(), "zh_CN.UTF-8".into()),
            ("LC_MESSAGES".into(), "zh_TW.UTF-8".into()),
            ("LC_ALL".into(), "C.UTF-8".into()),
        ]));
        assert_eq!(resolve_language(LanguageChoice::Auto, &env), UiLanguage::En);
    }

    #[test]
    fn prescan_accepts_equals_and_split_forms_anywhere() {
        assert_eq!(
            prescan_language(&[
                "agentic-gpt".into(),
                "config".into(),
                "--language=zh-CN".into()
            ])
            .unwrap(),
            LanguageChoice::ZhCn
        );
        assert_eq!(
            prescan_language(&[
                "agentic-gpt".into(),
                "config".into(),
                "init".into(),
                "--language".into(),
                "en".into(),
            ])
            .unwrap(),
            LanguageChoice::En
        );
    }

    #[test]
    fn every_catalog_entry_is_non_empty_for_each_language() {
        for language in [UiLanguage::ZhCn, UiLanguage::En] {
            let catalog = text(language);
            let entries = [
                catalog.app_about,
                catalog.config_about,
                catalog.config_init_about,
                catalog.config_keys_about,
                catalog.config_set_about,
                catalog.usage_heading,
                catalog.commands_heading,
                catalog.options_heading,
                catalog.arguments_heading,
                catalog.help_flag,
                catalog.version_flag,
                catalog.cancelled,
                catalog.initialized,
                catalog.replace_tunnel_id,
                catalog.provision_tunnel_secret,
                catalog.configure_hub_url,
                catalog.replace_agent_secret,
                catalog.optional_settings_prompt,
                catalog.safe_defaults_option,
                catalog.choose_sections_option,
            ];

            for entry in entries {
                assert!(!entry.is_empty());
            }
        }
    }
}
