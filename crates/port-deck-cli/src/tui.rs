use std::io::{self, IsTerminal, Stdout};
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use port_deck_cli::{
    Filters, StopTarget, StraydConfig, TabTarget, TuiArgs, apply_visibility_config,
    build_stop_plan, runtime_slug,
};
use port_deck_core::{HostPlatform, ResourceGroup, ResourceKind, ServiceProcess};
use port_deck_engine::{ScanSnapshot, scan_all, terminate_service};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Clear, List, ListItem, ListState, Padding, Paragraph, Tabs, Wrap,
};

const CYAN: Color = Color::Rgb(61, 214, 208);
const PURPLE: Color = Color::Rgb(171, 114, 255);
const MUTED: Color = Color::Rgb(117, 128, 151);
const PANEL: Color = Color::Rgb(15, 20, 31);
const SELECTED: Color = Color::Rgb(25, 40, 58);
const WARNING: Color = Color::Rgb(247, 190, 77);
const DANGER: Color = Color::Rgb(255, 101, 124);

pub fn run(args: TuiArgs, config: StraydConfig) -> Result<(), String> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("TUI 需要交互终端；脚本中请使用 list 或 stop 子命令".into());
    }

    enable_raw_mode().map_err(|error| format!("无法进入终端 raw mode: {error}"))?;
    let mut stdout = io::stdout();
    if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture) {
        let _ = disable_raw_mode();
        return Err(format!("无法进入终端备用屏幕: {error}"));
    }
    let mut terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
            return Err(format!("无法创建终端界面: {error}"));
        }
    };

    let result = App::new(args, config).and_then(|mut app| app.run(&mut terminal));
    let cleanup_result = restore_terminal(&mut terminal);
    result.and(cleanup_result)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<(), String> {
    disable_raw_mode().map_err(|error| format!("无法恢复终端模式: {error}"))?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )
    .map_err(|error| format!("无法离开终端备用屏幕: {error}"))?;
    terminal
        .show_cursor()
        .map_err(|error| format!("无法恢复终端光标: {error}"))
}

struct App {
    snapshot: ScanSnapshot,
    config: StraydConfig,
    tab: TabTarget,
    selected: usize,
    list_offset: usize,
    hidden_count: usize,
    regions: UiRegions,
    pending: Option<PendingStop>,
    status: String,
    refresh_every: Option<Duration>,
    last_refresh: Instant,
    should_quit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MouseAction {
    SetTab(TabTarget),
    Select(usize),
    StopDev,
    StopTunnel,
    StopGroup,
    Refresh,
    Quit,
    Confirm,
    Cancel,
}

#[derive(Debug, Clone)]
struct UiRegions {
    tabs: Vec<(Rect, TabTarget)>,
    list: Rect,
    footer_actions: Vec<(Rect, MouseAction)>,
    confirm: Rect,
    cancel: Rect,
}

struct PendingStop {
    label: String,
    services: Vec<ServiceProcess>,
}

impl App {
    fn new(args: TuiArgs, config: StraydConfig) -> Result<Self, String> {
        let (snapshot, hidden_count) = scan_visible(&config);
        let status = scan_status(&snapshot, hidden_count);
        Ok(Self {
            snapshot,
            config,
            tab: args.tab,
            selected: 0,
            list_offset: 0,
            hidden_count,
            regions: UiRegions::new(Rect::default()),
            pending: None,
            status,
            refresh_every: (args.refresh > 0).then(|| Duration::from_secs(args.refresh)),
            last_refresh: Instant::now(),
            should_quit: false,
        })
    }

    fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<(), String> {
        while !self.should_quit {
            terminal
                .draw(|frame| self.draw(frame))
                .map_err(|error| format!("终端渲染失败: {error}"))?;

            if event::poll(Duration::from_millis(200))
                .map_err(|error| format!("无法读取终端事件: {error}"))?
            {
                match event::read().map_err(|error| format!("无法读取终端事件: {error}"))? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => self.handle_key(key),
                    Event::Mouse(mouse) => self.handle_mouse(mouse),
                    _ => {}
                }
            }
            if self
                .refresh_every
                .is_some_and(|interval| self.last_refresh.elapsed() >= interval)
                && self.pending.is_none()
            {
                self.refresh();
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if self.pending.is_some() {
            match key.code {
                KeyCode::Enter => self.confirm_pending(),
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.pending = None;
                    self.status = "已取消关闭操作".into();
                }
                _ => {}
            }
            return;
        }

        if is_quit_key(key) {
            self.should_quit = true;
            return;
        }

        match key.code {
            KeyCode::Tab => self.next_tab(),
            KeyCode::BackTab => self.previous_tab(),
            KeyCode::Char('1') => self.set_tab(TabTarget::All),
            KeyCode::Char('2') => self.set_tab(TabTarget::Dev),
            KeyCode::Char('3') => self.set_tab(TabTarget::Tunnels),
            KeyCode::Char('4') => self.set_tab(TabTarget::System),
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_previous(),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.select_last(),
            KeyCode::Char('r') => self.refresh(),
            KeyCode::Char('d') => self.prepare_stop(StopTarget::Dev, "开发服务"),
            KeyCode::Char('t') => self.prepare_stop(StopTarget::Tunnel, "隧道"),
            KeyCode::Char('x') => self.prepare_stop(StopTarget::Group, "整个关联组"),
            _ => {}
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollDown if self.pending.is_none() => self.select_next(),
            MouseEventKind::ScrollUp if self.pending.is_none() => self.select_previous(),
            MouseEventKind::Down(MouseButton::Left) => {
                let visible_len = self.visible_group_indices().len();
                if let Some(action) = self.regions.action_at(
                    mouse,
                    self.list_offset,
                    visible_len,
                    self.pending.is_some(),
                ) {
                    self.handle_mouse_action(action);
                }
            }
            _ => {}
        }
    }

    fn handle_mouse_action(&mut self, action: MouseAction) {
        match action {
            MouseAction::SetTab(tab) => self.set_tab(tab),
            MouseAction::Select(index) => self.selected = index,
            MouseAction::StopDev => self.prepare_stop(StopTarget::Dev, "开发服务"),
            MouseAction::StopTunnel => self.prepare_stop(StopTarget::Tunnel, "隧道"),
            MouseAction::StopGroup => self.prepare_stop(StopTarget::Group, "整个关联组"),
            MouseAction::Refresh => self.refresh(),
            MouseAction::Quit => self.should_quit = true,
            MouseAction::Confirm => self.confirm_pending(),
            MouseAction::Cancel => {
                self.pending = None;
                self.status = "已取消关闭操作".into();
            }
        }
    }

    fn refresh(&mut self) {
        let selected_id = self.selected_group().map(|group| group.id.clone());
        (self.snapshot, self.hidden_count) = scan_visible(&self.config);
        self.last_refresh = Instant::now();
        self.status = scan_status(&self.snapshot, self.hidden_count);
        let visible = self.visible_group_indices();
        self.selected = selected_id
            .and_then(|id| {
                visible
                    .iter()
                    .position(|index| self.snapshot.groups[*index].id == id)
            })
            .unwrap_or_else(|| self.selected.min(visible.len().saturating_sub(1)));
    }

    fn prepare_stop(&mut self, target: StopTarget, label: &str) {
        let Some(group) = self.selected_group().cloned() else {
            self.status = "当前页面没有可操作资源".into();
            return;
        };
        match build_stop_plan(&[group], target, &Filters::default(), true) {
            Ok(services) => {
                self.pending = Some(PendingStop {
                    label: label.into(),
                    services,
                });
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn confirm_pending(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let mut stopped = 0;
        let mut failures = Vec::new();
        for service in &pending.services {
            match terminate_service(service) {
                Ok(()) => stopped += 1,
                Err(error) => failures.push(format!("pid {}: {error}", service.pid)),
            }
        }
        self.refresh();
        self.status = if failures.is_empty() {
            format!("已停止 {stopped} 项")
        } else {
            format!("已停止 {stopped} 项；{} 项失败", failures.len())
        };
    }

    fn next_tab(&mut self) {
        self.tab = match self.tab {
            TabTarget::All => TabTarget::Dev,
            TabTarget::Dev => TabTarget::Tunnels,
            TabTarget::Tunnels => TabTarget::System,
            TabTarget::System => TabTarget::All,
        };
        self.selected = 0;
        self.list_offset = 0;
    }

    fn previous_tab(&mut self) {
        self.tab = match self.tab {
            TabTarget::All => TabTarget::System,
            TabTarget::Dev => TabTarget::All,
            TabTarget::Tunnels => TabTarget::Dev,
            TabTarget::System => TabTarget::Tunnels,
        };
        self.selected = 0;
        self.list_offset = 0;
    }

    fn set_tab(&mut self, tab: TabTarget) {
        self.tab = tab;
        self.selected = 0;
        self.list_offset = 0;
    }

    fn select_next(&mut self) {
        let len = self.visible_group_indices().len();
        if len > 0 {
            self.selected = (self.selected + 1).min(len - 1);
        }
    }

    fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn select_last(&mut self) {
        self.selected = self.visible_group_indices().len().saturating_sub(1);
    }

    fn visible_group_indices(&self) -> Vec<usize> {
        self.snapshot
            .groups
            .iter()
            .enumerate()
            .filter(|(_, group)| tab_matches(self.tab, group))
            .map(|(index, _)| index)
            .collect()
    }

    fn selected_group(&self) -> Option<&ResourceGroup> {
        self.visible_group_indices()
            .get(self.selected)
            .and_then(|index| self.snapshot.groups.get(*index))
    }

    fn draw(&mut self, frame: &mut ratatui::Frame) {
        let area = frame.area();
        let sections = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(area);
        self.regions = UiRegions::new(area);

        self.draw_tabs(frame, sections[0]);
        self.draw_content(frame, sections[1]);
        self.draw_footer(frame, sections[2]);
        if let Some(pending) = &self.pending {
            self.draw_confirmation(frame, area, pending);
        }
    }

    fn draw_tabs(&self, frame: &mut ratatui::Frame, area: Rect) {
        let titles = ["1 ALL", "2 DEV", "3 TUNNELS", "4 SYSTEM"]
            .into_iter()
            .map(Line::from)
            .collect::<Vec<_>>();
        let selected = match self.tab {
            TabTarget::All => 0,
            TabTarget::Dev => 1,
            TabTarget::Tunnels => 2,
            TabTarget::System => 3,
        };
        let tabs = Tabs::new(titles)
            .select(selected)
            .block(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .title(" STRAYD / TERMINAL CONTROL "),
            )
            .style(Style::default().fg(MUTED))
            .highlight_style(Style::default().fg(CYAN).add_modifier(Modifier::BOLD))
            .divider("  ");
        frame.render_widget(tabs, area);
    }

    fn draw_content(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let columns = Layout::horizontal([Constraint::Percentage(43), Constraint::Percentage(57)])
            .split(area);
        let visible = self.visible_group_indices();
        let items = visible
            .iter()
            .map(|index| group_list_item(&self.snapshot.groups[*index]))
            .collect::<Vec<_>>();
        let list = List::new(items)
            .block(
                Block::bordered()
                    .title(format!(" RESOURCES / {} ", visible.len()))
                    .border_style(Style::default().fg(Color::DarkGray))
                    .padding(Padding::horizontal(1)),
            )
            .highlight_symbol("▌ ")
            .highlight_style(
                Style::default()
                    .fg(Color::White)
                    .bg(SELECTED)
                    .add_modifier(Modifier::BOLD),
            );
        let mut state = ListState::default()
            .with_selected((!visible.is_empty()).then_some(self.selected))
            .with_offset(self.list_offset);
        frame.render_stateful_widget(list, columns[0], &mut state);
        self.list_offset = state.offset();

        let detail = self
            .selected_group()
            .map(group_detail)
            .unwrap_or_else(|| Text::from("当前 Tab 没有资源"));
        let detail = Paragraph::new(detail)
            .block(
                Block::bordered()
                    .title(" RELATION & PROCESS DETAIL ")
                    .border_style(Style::default().fg(PURPLE))
                    .padding(Padding::horizontal(1)),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(detail, columns[1]);
    }

    fn draw_footer(&self, frame: &mut ratatui::Frame, area: Rect) {
        let action_row = Rect::new(area.x, area.y, area.width, 1);
        let action_areas = footer_action_areas(action_row);
        for (index, (label, key_label, danger)) in [
            ("SERVICE", "d", true),
            ("TUNNEL", "t", true),
            ("GROUP", "x", true),
            ("REFRESH", "r", false),
            ("QUIT", "q", false),
        ]
        .into_iter()
        .enumerate()
        {
            let color = if danger { DANGER } else { CYAN };
            let button = Paragraph::new(Line::from(vec![
                Span::styled(
                    format!(" {key_label} "),
                    Style::default().fg(Color::Black).bg(color).bold(),
                ),
                Span::styled(format!(" {label}"), Style::default().fg(color)),
            ]));
            frame.render_widget(button, action_areas[index]);
        }
        if let Some(hint_area) = action_areas.get(5) {
            frame.render_widget(
                Paragraph::new("mouse: tabs · rows · actions · wheel")
                    .style(Style::default().fg(MUTED)),
                *hint_area,
            );
        }
        let status = Line::from(Span::styled(
            format!("  {}", self.status),
            Style::default().fg(if self.snapshot.warnings.is_empty() {
                MUTED
            } else {
                WARNING
            }),
        ));
        let status_area = Rect::new(area.x, area.y.saturating_add(1), area.width, 1);
        frame.render_widget(Paragraph::new(status), status_area);
    }

    fn draw_confirmation(&self, frame: &mut ratatui::Frame, area: Rect, pending: &PendingStop) {
        let popup = centered_rect(58, 9, area);
        frame.render_widget(Clear, popup);
        let text = Text::from(vec![
            Line::from(Span::styled(
                format!("即将关闭 {} · {} 项", pending.label, pending.services.len()),
                Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("按 Enter 确认，Esc 取消").alignment(Alignment::Center),
        ]);
        let paragraph = Paragraph::new(text).alignment(Alignment::Center).block(
            Block::bordered()
                .border_type(BorderType::Double)
                .border_style(Style::default().fg(DANGER))
                .title(" CONFIRM STOP ")
                .style(Style::default().bg(PANEL)),
        );
        frame.render_widget(paragraph, popup);
        let [confirm, cancel] = confirmation_button_areas(popup);
        frame.render_widget(
            Paragraph::new(" ENTER / CONFIRM ")
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::Black).bg(DANGER).bold()),
            confirm,
        );
        frame.render_widget(
            Paragraph::new(" ESC / CANCEL ")
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::Black).bg(MUTED).bold()),
            cancel,
        );
    }
}

impl UiRegions {
    fn new(area: Rect) -> Self {
        let [tabs_area, content_area, footer_area] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .areas(area);
        let [list_column, _] =
            Layout::horizontal([Constraint::Percentage(43), Constraint::Percentage(57)])
                .areas(content_area);
        let list = Rect::new(
            list_column.x.saturating_add(2),
            list_column.y.saturating_add(1),
            list_column.width.saturating_sub(4),
            list_column.height.saturating_sub(2),
        );
        let tab_y = tabs_area.y.saturating_add(1);
        let mut tab_x = tabs_area.x.saturating_add(1);
        let tabs = [
            (5, TabTarget::All),
            (5, TabTarget::Dev),
            (9, TabTarget::Tunnels),
            (8, TabTarget::System),
        ]
        .into_iter()
        .map(|(width, tab)| {
            let rect = Rect::new(tab_x, tab_y, width, 1);
            tab_x = tab_x.saturating_add(width + 2);
            (rect, tab)
        })
        .collect();
        let action_row = Rect::new(footer_area.x, footer_area.y, footer_area.width, 1);
        let footer_areas = footer_action_areas(action_row);
        let footer_actions = [
            MouseAction::StopDev,
            MouseAction::StopTunnel,
            MouseAction::StopGroup,
            MouseAction::Refresh,
            MouseAction::Quit,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, action)| (footer_areas[index], action))
        .collect();
        let popup = centered_rect(58, 9, area);
        let [confirm, cancel] = confirmation_button_areas(popup);
        Self {
            tabs,
            list,
            footer_actions,
            confirm,
            cancel,
        }
    }

    fn action_at(
        &self,
        mouse: MouseEvent,
        list_offset: usize,
        visible_len: usize,
        confirmation_open: bool,
    ) -> Option<MouseAction> {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return None;
        }
        if confirmation_open {
            return rect_contains(self.confirm, mouse.column, mouse.row)
                .then_some(MouseAction::Confirm)
                .or_else(|| {
                    rect_contains(self.cancel, mouse.column, mouse.row)
                        .then_some(MouseAction::Cancel)
                });
        }
        if let Some((_, tab)) = self
            .tabs
            .iter()
            .find(|(area, _)| rect_contains(*area, mouse.column, mouse.row))
        {
            return Some(MouseAction::SetTab(*tab));
        }
        if rect_contains(self.list, mouse.column, mouse.row) {
            let item = list_offset + usize::from(mouse.row.saturating_sub(self.list.y) / 2);
            return (item < visible_len).then_some(MouseAction::Select(item));
        }
        self.footer_actions
            .iter()
            .find(|(area, _)| rect_contains(*area, mouse.column, mouse.row))
            .map(|(_, action)| *action)
    }
}

fn footer_action_areas(area: Rect) -> [Rect; 6] {
    Layout::horizontal([
        Constraint::Length(11),
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Length(11),
        Constraint::Length(8),
        Constraint::Min(0),
    ])
    .areas(area)
}

fn confirmation_button_areas(popup: Rect) -> [Rect; 2] {
    let row = Rect::new(popup.x, popup.y.saturating_add(6), popup.width, 1);
    let [confirm, _, cancel] = Layout::horizontal([
        Constraint::Length(18),
        Constraint::Length(2),
        Constraint::Length(18),
    ])
    .flex(Flex::Center)
    .areas(row);
    [confirm, cancel]
}

fn rect_contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

fn tab_matches(tab: TabTarget, group: &ResourceGroup) -> bool {
    match tab {
        TabTarget::All => true,
        TabTarget::Dev => group
            .services
            .iter()
            .any(|service| service.resource_kind == ResourceKind::Development),
        TabTarget::Tunnels => group
            .services
            .iter()
            .any(|service| service.resource_kind == ResourceKind::Tunnel),
        TabTarget::System => group
            .services
            .iter()
            .any(|service| service.resource_kind == ResourceKind::System),
    }
}

fn group_list_item(group: &ResourceGroup) -> ListItem<'static> {
    let primary = group
        .services
        .iter()
        .find(|service| service.resource_kind != ResourceKind::Tunnel)
        .or_else(|| group.services.first());
    let name = primary
        .and_then(|service| service.project_name.as_deref())
        .or_else(|| primary.map(|service| service.process_name.as_str()))
        .unwrap_or("unknown")
        .to_owned();
    let port = group
        .primary_port
        .map(|port| format!(":{port}"))
        .unwrap_or_else(|| "NO PORT".into());
    let tunnels = group
        .services
        .iter()
        .filter(|service| service.resource_kind == ResourceKind::Tunnel)
        .count();
    let suffix = if tunnels > 0 {
        format!("{} items / {} tunnel", group.services.len(), tunnels)
    } else {
        format!("{} item", group.services.len())
    };
    ListItem::new(vec![
        Line::from(vec![
            Span::styled(port, Style::default().fg(CYAN).bold()),
            Span::raw("  "),
            Span::styled(name, Style::default().fg(Color::White)),
        ]),
        Line::from(Span::styled(suffix, Style::default().fg(MUTED))),
    ])
}

fn group_detail(group: &ResourceGroup) -> Text<'static> {
    let mut lines = Vec::new();
    lines.push(route_line(group));
    lines.push(Line::from(""));
    for (index, service) in group.services.iter().enumerate() {
        let role = match service.resource_kind {
            ResourceKind::Development => "SERVICE",
            ResourceKind::Tunnel => "TUNNEL",
            ResourceKind::System => "SYSTEM",
            ResourceKind::Other => "PROCESS",
        };
        let role_color = match service.resource_kind {
            ResourceKind::Tunnel => PURPLE,
            ResourceKind::System => WARNING,
            _ => CYAN,
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {role} "),
                Style::default().fg(PANEL).bg(role_color).bold(),
            ),
            Span::raw("  "),
            Span::styled(
                runtime_slug(&service.runtime),
                Style::default().fg(Color::White).bold(),
            ),
            Span::styled(format!("  PID {}", service.pid), Style::default().fg(MUTED)),
        ]));
        lines.push(Line::from(format!("scope    {}", scope_label(service))));
        if !service.ports.is_empty() {
            lines.push(Line::from(format!(
                "listen   {}",
                service
                    .ports
                    .iter()
                    .map(|port| format!(":{port}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        if let Some(target) = &service.tunnel_target {
            lines.push(Line::from(format!(
                "proxy    {}:{}",
                target.host, target.port
            )));
        }
        if let Some(cwd) = &service.cwd {
            lines.push(Line::from(format!("cwd      {cwd}")));
        }
        if let Some(unit) = &service.manager_unit {
            lines.push(Line::from(format!("unit     {unit}")));
        }
        lines.push(Line::from(vec![
            Span::styled("command  ", Style::default().fg(MUTED)),
            Span::raw(service.command.clone()),
        ]));
        if !service.can_terminate {
            lines.push(Line::from(Span::styled(
                "PROTECTED · 不允许结束",
                Style::default().fg(WARNING).bold(),
            )));
        }
        if index + 1 < group.services.len() {
            lines.push(Line::from(Span::styled(
                "────────────────────────",
                Style::default().fg(Color::DarkGray),
            )));
        }
    }
    Text::from(lines)
}

fn route_line(group: &ResourceGroup) -> Line<'static> {
    let tunnel = group
        .services
        .iter()
        .find(|service| service.resource_kind == ResourceKind::Tunnel);
    let source = group
        .services
        .iter()
        .find(|service| service.resource_kind != ResourceKind::Tunnel);
    match (tunnel, source, group.primary_port) {
        (Some(tunnel), Some(source), Some(port)) => Line::from(vec![
            Span::styled("PUBLIC", Style::default().fg(PURPLE).bold()),
            Span::styled("  ──▶  ", Style::default().fg(MUTED)),
            Span::styled(runtime_slug(&tunnel.runtime), Style::default().fg(PURPLE)),
            Span::styled("  ──▶  ", Style::default().fg(MUTED)),
            Span::styled(
                format!(
                    ":{port} {}",
                    source
                        .project_name
                        .as_deref()
                        .unwrap_or(&source.process_name)
                ),
                Style::default().fg(CYAN).bold(),
            ),
        ]),
        (_, Some(source), Some(port)) => Line::from(Span::styled(
            format!(
                "LOCAL :{port} / {}",
                source
                    .project_name
                    .as_deref()
                    .unwrap_or(&source.process_name)
            ),
            Style::default().fg(CYAN).bold(),
        )),
        (Some(tunnel), _, Some(port)) => Line::from(Span::styled(
            format!("{} ──▶ localhost:{port}", runtime_slug(&tunnel.runtime)),
            Style::default().fg(PURPLE).bold(),
        )),
        _ => Line::from(Span::styled("UNBOUND RESOURCE", Style::default().fg(MUTED))),
    }
}

fn scope_label(service: &ServiceProcess) -> String {
    match service.platform {
        HostPlatform::Windows => "Windows".into(),
        HostPlatform::Linux => "Linux".into(),
        HostPlatform::MacOs => "macOS".into(),
    }
}

fn scan_visible(config: &StraydConfig) -> (ScanSnapshot, usize) {
    let mut snapshot = scan_all();
    let total = resource_count(&snapshot.groups);
    snapshot.groups = apply_visibility_config(&snapshot.groups, config);
    let hidden = total.saturating_sub(resource_count(&snapshot.groups));
    (snapshot, hidden)
}

fn resource_count(groups: &[ResourceGroup]) -> usize {
    groups.iter().map(|group| group.services.len()).sum()
}

fn scan_status(snapshot: &ScanSnapshot, hidden_count: usize) -> String {
    let resources = snapshot
        .groups
        .iter()
        .map(|group| group.services.len())
        .sum::<usize>();
    let hidden = if hidden_count > 0 {
        format!(" · {hidden_count} hidden by config")
    } else {
        String::new()
    };
    if snapshot.warnings.is_empty() {
        format!(
            "{} groups / {resources} resources · scan ready{hidden}",
            snapshot.groups.len(),
        )
    } else {
        format!(
            "{} groups / {resources} resources · {} warnings{hidden}",
            snapshot.groups.len(),
            snapshot.warnings.len()
        )
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let [vertical] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [horizontal] = Layout::horizontal([Constraint::Percentage(width)])
        .flex(Flex::Center)
        .areas(vertical);
    horizontal
}

fn is_quit_key(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q')
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}

#[cfg(test)]
mod tests {
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use ratatui::layout::Rect;

    use super::{MouseAction, UiRegions, is_quit_key};
    use port_deck_cli::TabTarget;

    #[test]
    fn plain_q_and_control_c_are_quit_keys() {
        assert!(is_quit_key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE
        )));
        assert!(is_quit_key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
        assert!(!is_quit_key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::NONE
        )));
    }

    #[test]
    fn mouse_clicks_map_to_tabs_rows_and_footer_actions() {
        let regions = UiRegions::new(Rect::new(0, 0, 100, 30));

        assert_eq!(
            regions.action_at(mouse_down(3, 1), 0, 8, false),
            Some(MouseAction::SetTab(TabTarget::All))
        );
        assert_eq!(
            regions.action_at(mouse_down(17, 1), 0, 8, false),
            Some(MouseAction::SetTab(TabTarget::Tunnels))
        );
        assert_eq!(
            regions.action_at(mouse_down(8, 8), 2, 8, false),
            Some(MouseAction::Select(4))
        );
        assert_eq!(
            regions.action_at(mouse_down(13, 28), 0, 8, false),
            Some(MouseAction::StopTunnel)
        );
        assert_eq!(
            regions.action_at(mouse_down(45, 28), 0, 8, false),
            Some(MouseAction::Quit)
        );
    }

    #[test]
    fn confirmation_dialog_captures_mouse_clicks() {
        let regions = UiRegions::new(Rect::new(0, 0, 100, 30));

        assert_eq!(
            regions.action_at(mouse_down(38, 17), 0, 8, true),
            Some(MouseAction::Confirm)
        );
        assert_eq!(
            regions.action_at(mouse_down(57, 17), 0, 8, true),
            Some(MouseAction::Cancel)
        );
        assert_eq!(regions.action_at(mouse_down(3, 1), 0, 8, true), None);
    }

    fn mouse_down(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }
}
