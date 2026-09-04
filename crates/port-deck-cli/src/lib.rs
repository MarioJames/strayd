use clap::{Args, Parser, Subcommand, ValueEnum};
use port_deck_core::{HostPlatform, ResourceGroup, ResourceKind, RuntimeKind, ServiceProcess};

#[derive(Debug, Parser)]
#[command(
    name = "strayd",
    version,
    about = "Manage local development services and tunnels"
)]
pub struct Cli {
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
    #[error("没有资源匹配当前筛选条件")]
    NoMatches,
    #[error("匹配到 {0} 项；批量操作需要显式传入 --all")]
    MultipleMatches(usize),
    #[error("资源组 {0} 包含受保护服务，拒绝整组关闭")]
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
