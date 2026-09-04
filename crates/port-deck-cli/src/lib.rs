use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};
use directories::BaseDirs;
use port_deck_core::{HostPlatform, ResourceGroup, ResourceKind, RuntimeKind, ServiceProcess};
use serde::{Deserialize, Serialize};

mod i18n;

pub use i18n::{Language, Translator};

pub const DEFAULT_CONFIG_TOML: &str = r#"# Strayd configuration
# Rules are ORed; fields inside one rule are ANDed.
version = 1
language = "auto"

# Example: hide only the sshd resource listening on port 22.
# [[display.hide]]
# ports = [22]
# runtimes = ["sshd"]

# Other matchers: kinds, platforms, process_names, projects, commands, ids.
"#;

#[derive(Debug, Parser)]
#[command(
    name = "strayd",
    version,
    about = "Manage local development services and tunnels"
)]
pub struct Cli {
    /// Override the display language (auto, en, or zh-cn)
    #[arg(long, global = true, value_enum)]
    pub language: Option<LanguageSetting>,

    /// Use a specific configuration file
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Ignore the persisted configuration for this invocation
    #[arg(long, global = true, conflicts_with = "config")]
    pub no_config: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Open the interactive terminal interface
    Tui(TuiArgs),
    /// List detected resources
    List(ListArgs),
    /// Stop development services, tunnels, groups, or individual resources
    Stop(StopArgs),
    /// Inspect or initialize persistent configuration
    Config(ConfigArgs),
}

#[derive(Debug, Clone, Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub action: ConfigAction,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ConfigAction {
    /// Print the resolved configuration path
    Path,
    /// Create a documented configuration file
    Init {
        /// Replace an existing configuration file
        #[arg(long)]
        force: bool,
    },
    /// Print and validate the effective configuration
    Show,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StraydConfig {
    #[serde(default = "current_config_version")]
    pub version: u32,
    pub language: LanguageSetting,
    pub display: DisplayConfig,
}

impl Default for StraydConfig {
    fn default() -> Self {
        Self {
            version: current_config_version(),
            language: LanguageSetting::Auto,
            display: DisplayConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum LanguageSetting {
    #[default]
    Auto,
    #[value(name = "en", alias = "en-us")]
    #[serde(rename = "en", alias = "en-us")]
    English,
    #[value(name = "zh-cn", alias = "zh")]
    #[serde(rename = "zh-cn", alias = "zh")]
    ZhCn,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DisplayConfig {
    pub hide: Vec<HideRule>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HideRule {
    pub ports: Vec<u16>,
    pub runtimes: Vec<String>,
    pub kinds: Vec<ConfigResourceKind>,
    pub platforms: Vec<ConfigPlatform>,
    pub process_names: Vec<String>,
    pub projects: Vec<String>,
    pub commands: Vec<String>,
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HideField {
    Port,
    Runtime,
    Kind,
    Platform,
    ProcessName,
    Project,
    Command,
    Id,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigResourceKind {
    #[serde(alias = "development")]
    Dev,
    Tunnel,
    System,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigPlatform {
    Windows,
    Linux,
    Macos,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config_directory_unavailable")]
    DirectoryUnavailable,
    #[error("config_read_failed: {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("config_parse_failed: {path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("config_version_unsupported: {0}")]
    UnsupportedVersion(u32),
    #[error("config_already_exists: {0}")]
    AlreadyExists(PathBuf),
    #[error("config_write_failed: {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

const fn current_config_version() -> u32 {
    1
}

pub fn default_config_path() -> Result<PathBuf, ConfigError> {
    BaseDirs::new()
        .map(|dirs| dirs.config_dir().join("strayd").join("config.toml"))
        .ok_or(ConfigError::DirectoryUnavailable)
}

pub fn resolve_config_path(explicit: Option<&Path>) -> Result<PathBuf, ConfigError> {
    explicit
        .map(Path::to_path_buf)
        .map(Ok)
        .unwrap_or_else(default_config_path)
}

pub fn load_config(path: &Path) -> Result<StraydConfig, ConfigError> {
    match fs::read_to_string(path) {
        Ok(contents) => parse_config_at(&contents, path),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(StraydConfig::default()),
        Err(source) => Err(ConfigError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

pub fn parse_config(contents: &str) -> Result<StraydConfig, ConfigError> {
    parse_config_at(contents, Path::new("<memory>"))
}

fn parse_config_at(contents: &str, path: &Path) -> Result<StraydConfig, ConfigError> {
    let config = toml::from_str::<StraydConfig>(contents).map_err(|error| ConfigError::Parse {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    if config.version != current_config_version() {
        return Err(ConfigError::UnsupportedVersion(config.version));
    }
    Ok(config)
}

pub fn format_config(config: &StraydConfig) -> Result<String, ConfigError> {
    toml::to_string_pretty(config).map_err(|error| ConfigError::Parse {
        path: PathBuf::from("<effective-config>"),
        message: error.to_string(),
    })
}

pub fn initialize_config(path: &Path, force: bool) -> Result<(), ConfigError> {
    if path.exists() && !force {
        return Err(ConfigError::AlreadyExists(path.to_path_buf()));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(force);
    if !force {
        options.create_new(true);
    }
    let mut file = options.open(path).map_err(|source| ConfigError::Write {
        path: path.to_path_buf(),
        source,
    })?;
    file.write_all(DEFAULT_CONFIG_TOML.as_bytes())
        .map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })
}

pub fn save_config(path: &Path, config: &StraydConfig) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })?;
    }
    let contents = format_config(config)?;
    fs::write(path, contents).map_err(|source| ConfigError::Write {
        path: path.to_path_buf(),
        source,
    })
}

pub fn resolve_language(
    override_setting: Option<LanguageSetting>,
    configured: LanguageSetting,
    system_locale: Option<&str>,
    system_timezone: Option<&str>,
) -> Language {
    let setting = override_setting.unwrap_or(configured);
    match setting {
        LanguageSetting::English => Language::English,
        LanguageSetting::ZhCn => Language::ZhCn,
        LanguageSetting::Auto => detect_language(system_locale, system_timezone),
    }
}

pub fn resolve_system_language(
    override_setting: Option<LanguageSetting>,
    configured: LanguageSetting,
) -> Language {
    let locale = sys_locale::get_locale();
    let timezone = iana_time_zone::get_timezone().ok();
    resolve_language(
        override_setting,
        configured,
        locale.as_deref(),
        timezone.as_deref(),
    )
}

fn detect_language(system_locale: Option<&str>, system_timezone: Option<&str>) -> Language {
    if let Some(locale) = system_locale
        .map(str::trim)
        .filter(|locale| !locale.is_empty())
    {
        let normalized = locale
            .split(['.', '@'])
            .next()
            .unwrap_or(locale)
            .replace('_', "-")
            .to_ascii_lowercase();
        if normalized == "zh" || normalized.starts_with("zh-") {
            return Language::ZhCn;
        }
        if !matches!(normalized.as_str(), "c" | "posix") {
            return Language::English;
        }
    }

    let chinese_timezone = system_timezone.is_some_and(|timezone| {
        matches!(
            timezone.to_ascii_lowercase().as_str(),
            "asia/shanghai"
                | "asia/chongqing"
                | "asia/harbin"
                | "asia/urumqi"
                | "asia/hong_kong"
                | "asia/macau"
                | "asia/taipei"
                | "prc"
                | "hongkong"
                | "roc"
        )
    });
    if chinese_timezone {
        Language::ZhCn
    } else {
        Language::English
    }
}

pub fn apply_visibility_config(
    groups: &[ResourceGroup],
    config: &StraydConfig,
) -> Vec<ResourceGroup> {
    groups
        .iter()
        .filter_map(|group| {
            let services = group
                .services
                .iter()
                .filter(|service| !config.display.hide.iter().any(|rule| rule.matches(service)))
                .cloned()
                .collect::<Vec<_>>();
            (!services.is_empty()).then(|| ResourceGroup {
                id: group.id.clone(),
                primary_port: visible_primary_port(&services),
                services,
            })
        })
        .collect()
}

impl HideRule {
    pub fn from_service(service: &ServiceProcess, fields: &[HideField]) -> Option<Self> {
        let mut rule = Self::default();
        for field in fields {
            match field {
                HideField::Port => {
                    rule.ports.extend(service.ports.iter().copied());
                    if let Some(target) = &service.tunnel_target {
                        rule.ports.push(target.port);
                    }
                    rule.ports.sort_unstable();
                    rule.ports.dedup();
                }
                HideField::Runtime => rule.runtimes.push(runtime_slug(&service.runtime).into()),
                HideField::Kind => rule
                    .kinds
                    .push(ConfigResourceKind::from(&service.resource_kind)),
                HideField::Platform => rule.platforms.push(ConfigPlatform::from(service.platform)),
                HideField::ProcessName => rule.process_names.push(service.process_name.clone()),
                HideField::Project => {
                    if let Some(project) = service.project_name.as_ref().or(service.cwd.as_ref()) {
                        rule.projects.push(project.clone());
                    }
                }
                HideField::Command => rule.commands.push(service.command.clone()),
                HideField::Id => rule.ids.push(service.id.clone()),
            }
        }
        rule.has_matcher().then_some(rule)
    }

    pub fn has_matcher(&self) -> bool {
        !self.ports.is_empty()
            || !self.runtimes.is_empty()
            || !self.kinds.is_empty()
            || !self.platforms.is_empty()
            || !self.process_names.is_empty()
            || !self.projects.is_empty()
            || !self.commands.is_empty()
            || !self.ids.is_empty()
    }

    fn matches(&self, service: &ServiceProcess) -> bool {
        self.has_matcher()
            && (self.ports.is_empty()
                || self
                    .ports
                    .iter()
                    .any(|port| service_matches_port(service, *port)))
            && (self.runtimes.is_empty()
                || self
                    .runtimes
                    .iter()
                    .any(|runtime| runtime_slug(&service.runtime).eq_ignore_ascii_case(runtime)))
            && (self.kinds.is_empty()
                || self
                    .kinds
                    .iter()
                    .any(|kind| kind.matches(&service.resource_kind)))
            && (self.platforms.is_empty()
                || self
                    .platforms
                    .iter()
                    .any(|platform| platform.matches(service.platform)))
            && contains_config_value(&self.process_names, &service.process_name)
            && self.projects_match(service)
            && contains_config_value(&self.commands, &service.command)
            && (self.ids.is_empty()
                || self
                    .ids
                    .iter()
                    .any(|id| service.id.eq_ignore_ascii_case(id)))
    }

    fn projects_match(&self, service: &ServiceProcess) -> bool {
        self.projects.is_empty()
            || self.projects.iter().any(|expected| {
                service
                    .project_name
                    .as_deref()
                    .is_some_and(|value| contains_case_insensitive(value, expected))
                    || service
                        .cwd
                        .as_deref()
                        .is_some_and(|value| contains_case_insensitive(value, expected))
            })
    }
}

impl From<&ResourceKind> for ConfigResourceKind {
    fn from(kind: &ResourceKind) -> Self {
        match kind {
            ResourceKind::Development => Self::Dev,
            ResourceKind::Tunnel => Self::Tunnel,
            ResourceKind::System => Self::System,
            ResourceKind::Other => Self::Other,
        }
    }
}

impl From<HostPlatform> for ConfigPlatform {
    fn from(platform: HostPlatform) -> Self {
        match platform {
            HostPlatform::Windows => Self::Windows,
            HostPlatform::Linux => Self::Linux,
            HostPlatform::MacOs => Self::Macos,
        }
    }
}

impl ConfigResourceKind {
    fn matches(self, kind: &ResourceKind) -> bool {
        matches!(
            (self, kind),
            (Self::Dev, ResourceKind::Development)
                | (Self::Tunnel, ResourceKind::Tunnel)
                | (Self::System, ResourceKind::System)
                | (Self::Other, ResourceKind::Other)
        )
    }
}

impl ConfigPlatform {
    fn matches(self, platform: HostPlatform) -> bool {
        matches!(
            (self, platform),
            (Self::Windows, HostPlatform::Windows)
                | (Self::Linux, HostPlatform::Linux)
                | (Self::Macos, HostPlatform::MacOs)
        )
    }
}

fn contains_config_value(expected: &[String], actual: &str) -> bool {
    expected.is_empty()
        || expected
            .iter()
            .any(|value| contains_case_insensitive(actual, value))
}

fn contains_case_insensitive(actual: &str, expected: &str) -> bool {
    actual
        .to_ascii_lowercase()
        .contains(&expected.to_ascii_lowercase())
}

fn visible_primary_port(services: &[ServiceProcess]) -> Option<u16> {
    services
        .iter()
        .find_map(|service| service.tunnel_target.as_ref().map(|target| target.port))
        .or_else(|| {
            services
                .iter()
                .find_map(|service| service.ports.first().copied())
        })
}

#[derive(Debug, Clone, Args)]
pub struct TuiArgs {
    /// Initial tab
    #[arg(long, value_enum, default_value_t = TabTarget::All)]
    pub tab: TabTarget,

    /// Automatic refresh interval in seconds; use 0 to disable
    #[arg(long, default_value_t = 5)]
    pub refresh: u64,
}

impl Default for TuiArgs {
    fn default() -> Self {
        Self {
            tab: TabTarget::All,
            refresh: 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TabTarget {
    All,
    Dev,
    Tunnels,
    System,
    Config,
}

#[derive(Debug, Clone, Args)]
pub struct ListArgs {
    #[command(flatten)]
    pub filters: Filters,

    /// Limit results by resource kind
    #[arg(long, value_enum)]
    pub kind: Option<ListKind>,

    /// Emit machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ListKind {
    Dev,
    Tunnel,
    System,
    Other,
}

#[derive(Debug, Clone, Args)]
pub struct StopArgs {
    /// Resource category to stop
    #[arg(value_enum)]
    pub target: StopTarget,

    #[command(flatten)]
    pub filters: Filters,

    /// Allow an operation that matches more than one resource or group
    #[arg(long)]
    pub all: bool,

    /// Skip the confirmation prompt
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Print the stop plan without terminating anything
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum StopTarget {
    Dev,
    Tunnel,
    Group,
    Resource,
}

#[derive(Debug, Clone, Default, Args)]
pub struct Filters {
    /// Match a group or resource id
    #[arg(long)]
    pub id: Option<String>,

    /// Match a listening port or tunnel origin port
    #[arg(long, short = 'p')]
    pub port: Option<u16>,

    /// Match a project name or working directory
    #[arg(long)]
    pub project: Option<String>,

    /// Match the host platform
    #[arg(long, value_enum)]
    pub platform: Option<PlatformFilter>,

    /// Match a runtime such as next-js, vite, or cloudflared
    #[arg(long)]
    pub runtime: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PlatformFilter {
    Windows,
    Linux,
    Macos,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("plan_no_matches")]
    NoMatches,
    #[error("plan_multiple_matches: {0}")]
    MultipleMatches(usize),
    #[error("plan_protected_group: {0}")]
    ProtectedGroup(String),
}

pub fn build_stop_plan(
    groups: &[ResourceGroup],
    target: StopTarget,
    filters: &Filters,
    allow_many: bool,
) -> Result<Vec<ServiceProcess>, PlanError> {
    if target == StopTarget::Group {
        return build_group_stop_plan(groups, filters, allow_many);
    }

    let mut matched = groups
        .iter()
        .flat_map(|group| &group.services)
        .filter(|service| target_matches_service(target, service))
        .filter(|service| filters_match_service(filters, service))
        .cloned()
        .collect::<Vec<_>>();

    if matched.is_empty() {
        return Err(PlanError::NoMatches);
    }
    if matched.len() > 1 && !allow_many {
        return Err(PlanError::MultipleMatches(matched.len()));
    }
    matched.sort_by_key(stop_order);
    Ok(matched)
}

fn build_group_stop_plan(
    groups: &[ResourceGroup],
    filters: &Filters,
    allow_many: bool,
) -> Result<Vec<ServiceProcess>, PlanError> {
    let matched = groups
        .iter()
        .filter(|group| filters_match_group(filters, group))
        .collect::<Vec<_>>();

    if matched.is_empty() {
        return Err(PlanError::NoMatches);
    }
    if matched.len() > 1 && !allow_many {
        return Err(PlanError::MultipleMatches(matched.len()));
    }
    if let Some(group) = matched
        .iter()
        .find(|group| group.services.iter().any(|service| !service.can_terminate))
    {
        return Err(PlanError::ProtectedGroup(group.id.clone()));
    }

    let mut services = matched
        .into_iter()
        .flat_map(|group| group.services.iter().cloned())
        .collect::<Vec<_>>();
    services.sort_by_key(stop_order);
    Ok(services)
}

pub fn filter_groups(
    groups: &[ResourceGroup],
    filters: &Filters,
    kind: Option<ListKind>,
) -> Vec<ResourceGroup> {
    groups
        .iter()
        .filter(|group| filters_match_group(filters, group))
        .filter(|group| {
            kind.is_none_or(|kind| {
                group
                    .services
                    .iter()
                    .any(|service| list_kind_matches(kind, &service.resource_kind))
            })
        })
        .cloned()
        .collect()
}

fn filters_match_group(filters: &Filters, group: &ResourceGroup) -> bool {
    let id_matches = filters.id.as_ref().is_none_or(|expected| {
        group.id == *expected || group.services.iter().any(|service| service.id == *expected)
    });
    let port_matches = filters.port.is_none_or(|expected| {
        group.primary_port == Some(expected)
            || group
                .services
                .iter()
                .any(|service| service_matches_port(service, expected))
    });
    id_matches
        && port_matches
        && group
            .services
            .iter()
            .any(|service| filters_match_service_except_id_and_port(filters, service))
}

fn filters_match_service(filters: &Filters, service: &ServiceProcess) -> bool {
    filters
        .id
        .as_ref()
        .is_none_or(|expected| service.id == *expected)
        && filters
            .port
            .is_none_or(|expected| service_matches_port(service, expected))
        && filters_match_service_except_id_and_port(filters, service)
}

fn filters_match_service_except_id_and_port(filters: &Filters, service: &ServiceProcess) -> bool {
    filters.project.as_ref().is_none_or(|expected| {
        let expected = expected.to_ascii_lowercase();
        service
            .project_name
            .as_deref()
            .is_some_and(|value| value.to_ascii_lowercase().contains(&expected))
            || service
                .cwd
                .as_deref()
                .is_some_and(|value| value.to_ascii_lowercase().contains(&expected))
    }) && filters.platform.is_none_or(|platform| {
        matches!(
            (platform, service.platform),
            (PlatformFilter::Windows, HostPlatform::Windows)
                | (PlatformFilter::Linux, HostPlatform::Linux)
                | (PlatformFilter::Macos, HostPlatform::MacOs)
        )
    }) && filters
        .runtime
        .as_ref()
        .is_none_or(|expected| runtime_slug(&service.runtime).eq_ignore_ascii_case(expected))
}

fn service_matches_port(service: &ServiceProcess, expected: u16) -> bool {
    service.ports.contains(&expected)
        || service
            .tunnel_target
            .as_ref()
            .is_some_and(|target| target.port == expected)
}

fn target_matches_service(target: StopTarget, service: &ServiceProcess) -> bool {
    match target {
        StopTarget::Dev => service.resource_kind == ResourceKind::Development,
        StopTarget::Tunnel => service.resource_kind == ResourceKind::Tunnel,
        StopTarget::Resource => service.can_terminate,
        StopTarget::Group => false,
    }
}

fn list_kind_matches(kind: ListKind, resource_kind: &ResourceKind) -> bool {
    matches!(
        (kind, resource_kind),
        (ListKind::Dev, ResourceKind::Development)
            | (ListKind::Tunnel, ResourceKind::Tunnel)
            | (ListKind::System, ResourceKind::System)
            | (ListKind::Other, ResourceKind::Other)
    )
}

fn stop_order(service: &ServiceProcess) -> (u8, u32) {
    let order = match service.resource_kind {
        ResourceKind::Tunnel => 0,
        ResourceKind::Development => 1,
        ResourceKind::Other => 2,
        ResourceKind::System => 3,
    };
    (order, service.pid)
}

pub fn runtime_slug(runtime: &RuntimeKind) -> &'static str {
    match runtime {
        RuntimeKind::NextJs => "next-js",
        RuntimeKind::Vite => "vite",
        RuntimeKind::Nuxt => "nuxt",
        RuntimeKind::Astro => "astro",
        RuntimeKind::SvelteKit => "svelte-kit",
        RuntimeKind::Remix => "remix",
        RuntimeKind::Angular => "angular",
        RuntimeKind::Storybook => "storybook",
        RuntimeKind::Webpack => "webpack",
        RuntimeKind::Parcel => "parcel",
        RuntimeKind::Rspack => "rspack",
        RuntimeKind::Node => "node",
        RuntimeKind::Bun => "bun",
        RuntimeKind::Deno => "deno",
        RuntimeKind::Cloudflared => "cloudflared",
        RuntimeKind::Ngrok => "ngrok",
        RuntimeKind::SshTunnel => "ssh-tunnel",
        RuntimeKind::Frp => "frp",
        RuntimeKind::LocalTunnel => "local-tunnel",
        RuntimeKind::Bore => "bore",
        RuntimeKind::Sshd => "sshd",
        RuntimeKind::Other => "other",
    }
}
