use std::io::{self, IsTerminal, Stdout, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use port_deck_cli::{
    ConfigPlatform, ConfigResourceKind, Filters, HideField, HideRule, Language, StopTarget,
    StraydConfig, TabTarget, Translator, TuiArgs, apply_visibility_config, build_stop_plan,
    format_started_at, format_uptime, runtime_slug, save_config, unix_now,
};
use port_deck_core::{HostPlatform, ResourceGroup, ResourceKind, ServiceProcess};
use port_deck_engine::{ScanSnapshot, scan_all, terminate_service};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const CYAN: Color = Color::Rgb(61, 214, 208);
const PURPLE: Color = Color::Rgb(171, 114, 255);
const MUTED: Color = Color::Rgb(117, 128, 151);
const BASE: Color = Color::Rgb(7, 11, 18);
const SURFACE: Color = Color::Rgb(11, 16, 25);
const PANEL: Color = Color::Rgb(15, 20, 31);
const SELECTED: Color = Color::Rgb(25, 40, 58);
const BORDER: Color = Color::Rgb(53, 64, 80);
const TEXT: Color = Color::Rgb(247, 249, 252);
const WARNING: Color = Color::Rgb(247, 190, 77);
const DANGER: Color = Color::Rgb(255, 101, 124);
const GROUP_ITEM_HEIGHT: u16 = 3;
const RULE_ITEM_HEIGHT: u16 = 3;
const TAB_HEIGHT: u16 = 3;
const FOOTER_HEIGHT: u16 = 2;
const FOOTER_ACTION_HEIGHT: u16 = 1;
const MAIN_PANEL_TOP_PADDING: u16 = 1;
const MAIN_PANEL_PADDING: Padding = Padding::new(1, 1, MAIN_PANEL_TOP_PADDING, 0);
const PANEL_TITLE_LEFT_INSET: u16 = 3;

pub fn run(
    args: TuiArgs,
    config: StraydConfig,
    config_path: Option<PathBuf>,
    language: Language,
) -> Result<(), String> {
    let tr = Translator::new(language);
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(tr.text("tui_requires_terminal").into());
    }

    enable_raw_mode()
        .map_err(|error| tr.format("tui_raw_mode_error", &[("error", error.to_string())]))?;
    let mut stdout = io::stdout();
    if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture) {
        let _ = disable_raw_mode();
        return Err(tr.format("tui_alt_screen_error", &[("error", error.to_string())]));
    }
    let mut terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
            return Err(tr.format("tui_create_error", &[("error", error.to_string())]));
        }
    };

    let result = App::new(args, config, config_path, tr).and_then(|mut app| app.run(&mut terminal));
    let cleanup_result = restore_terminal(&mut terminal, tr);
    result.and(cleanup_result)
}

fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    tr: Translator,
) -> Result<(), String> {
    disable_raw_mode()
        .map_err(|error| tr.format("tui_restore_mode_error", &[("error", error.to_string())]))?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )
    .map_err(|error| tr.format("tui_leave_screen_error", &[("error", error.to_string())]))?;
    terminal
        .show_cursor()
        .map_err(|error| tr.format("tui_restore_cursor_error", &[("error", error.to_string())]))
}

struct App {
    snapshot: ScanSnapshot,
    config: StraydConfig,
    config_path: Option<PathBuf>,
    tr: Translator,
    tab: TabTarget,
    selected: usize,
    list_offset: usize,
    detail_scroll: u16,
    detail_group_id: Option<String>,
    hidden_count: usize,
    regions: UiRegions,
    pending: Option<PendingStop>,
    pending_hide: Option<PendingHide>,
    settings_open: bool,
    settings_selected: usize,
    settings_list_offset: usize,
    text_selection_mode: bool,
    status: String,
    refresh_every: Option<Duration>,
    last_refresh: Instant,
    should_quit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MouseAction {
    SetTab(TabTarget),
    Select(usize),
    SelectSetting(usize),
    Stop,
    Hide,
    OpenSettings,
    CloseSettings,
    SelectText,
    Refresh,
    Quit,
    CopyCommand(usize),
    RemoveRule,
    Confirm,
    Cancel,
    ToggleHideField(usize),
    SaveHide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavigationAction {
    NextTab,
    PreviousTab,
    NextItem,
    PreviousItem,
}

#[derive(Debug, Clone)]
struct UiRegions {
    tabs: Vec<(Rect, TabTarget)>,
    list: Rect,
    detail: Rect,
    footer_actions: Vec<(Rect, MouseAction)>,
    confirm: Rect,
    cancel: Rect,
    hide_fields: Vec<Rect>,
    hide_save: Rect,
    hide_cancel: Rect,
    settings_list: Rect,
    settings_list_panel: Rect,
    settings_detail_panel: Rect,
    settings_popup: Rect,
    settings_remove: Rect,
    settings_close: Rect,
    command_copies: Vec<(Rect, String)>,
}

struct DetailContent {
    text: Text<'static>,
    commands: Vec<(u16, String)>,
}

struct PendingStop {
    label: String,
    services: Vec<ServiceProcess>,
}

struct PendingHide {
    service: ServiceProcess,
    fields: Vec<HideChoice>,
    selected: usize,
}

struct HideChoice {
    field: HideField,
    value: String,
    available: bool,
    checked: bool,
}

impl App {
    fn new(
        args: TuiArgs,
        config: StraydConfig,
        config_path: Option<PathBuf>,
        tr: Translator,
    ) -> Result<Self, String> {
        let (snapshot, hidden_count) = scan_visible(&config);
        let status = scan_status(&snapshot, hidden_count, tr);
        Ok(Self {
            snapshot,
            config,
            config_path,
            tr,
            tab: args.tab,
            selected: 0,
            list_offset: 0,
            detail_scroll: 0,
            detail_group_id: None,
            hidden_count,
            regions: UiRegions::new(Rect::default(), args.tab, 0),
            pending: None,
            pending_hide: None,
            settings_open: false,
            settings_selected: 0,
            settings_list_offset: 0,
            text_selection_mode: false,
            status,
            refresh_every: (args.refresh > 0).then(|| Duration::from_secs(args.refresh)),
            last_refresh: Instant::now(),
            should_quit: false,
        })
    }

    fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<(), String> {
        while !self.should_quit {
            if !self.text_selection_mode {
                terminal.draw(|frame| self.draw(frame)).map_err(|error| {
                    self.tr
                        .format("tui_render_error", &[("error", error.to_string())])
                })?;
            }

            if event::poll(Duration::from_millis(200)).map_err(|error| {
                self.tr
                    .format("tui_event_error", &[("error", error.to_string())])
            })? {
                let input = event::read().map_err(|error| {
                    self.tr
                        .format("tui_event_error", &[("error", error.to_string())])
                })?;
                let was_selecting_text = self.text_selection_mode;
                match input {
                    Event::Key(key) if key.kind == KeyEventKind::Press => self.handle_key(key),
                    Event::Mouse(mouse) => self.handle_mouse(mouse),
                    _ => {}
                }
                if !was_selecting_text && self.text_selection_mode {
                    terminal.draw(|frame| self.draw(frame)).map_err(|error| {
                        self.tr
                            .format("tui_render_error", &[("error", error.to_string())])
                    })?;
                    execute!(terminal.backend_mut(), DisableMouseCapture).map_err(|error| {
                        self.tr
                            .format("tui_mouse_mode_error", &[("error", error.to_string())])
                    })?;
                } else if was_selecting_text && !self.text_selection_mode {
                    execute!(terminal.backend_mut(), EnableMouseCapture).map_err(|error| {
                        self.tr
                            .format("tui_mouse_mode_error", &[("error", error.to_string())])
                    })?;
                }
            }
            if self
                .refresh_every
                .is_some_and(|interval| self.last_refresh.elapsed() >= interval)
                && self.pending.is_none()
                && self.pending_hide.is_none()
                && !self.settings_open
                && !self.text_selection_mode
            {
                self.refresh();
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if self.text_selection_mode {
            if is_quit_key(key) {
                self.should_quit = true;
            } else if text_selection_transition(true, key.code) == Some(false) {
                self.text_selection_mode = false;
                self.status = self.tr.text("tui_text_selection_closed").into();
            }
            return;
        }
        if self.pending_hide.is_some() {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => self.select_next_hide_field(),
                KeyCode::Up | KeyCode::Char('k') => self.select_previous_hide_field(),
                KeyCode::Char(' ') => self.toggle_selected_hide_field(),
                KeyCode::Enter => self.save_pending_hide(),
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.pending_hide = None;
                    self.status = self.tr.text("tui_cancelled").into();
                }
                _ => {}
            }
            return;
        }
        if self.pending.is_some() {
            match key.code {
                KeyCode::Enter => self.confirm_pending(),
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.pending = None;
                    self.status = self.tr.text("tui_cancelled").into();
                }
                _ => {}
            }
            return;
        }
        if self.settings_open {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => self.select_next_setting(),
                KeyCode::Up | KeyCode::Char('k') => self.select_previous_setting(),
                KeyCode::Char('u') | KeyCode::Delete => self.remove_selected_rule(),
                KeyCode::Esc | KeyCode::Char(',') | KeyCode::Char('q') => {
                    self.settings_open = false;
                    self.status = self.tr.text("tui_settings_closed").into();
                }
                _ => {}
            }
            return;
        }

        if is_quit_key(key) {
            self.should_quit = true;
            return;
        }

        if let Some(action) = navigation_action(key.code) {
            match action {
                NavigationAction::NextTab => self.next_tab(),
                NavigationAction::PreviousTab => self.previous_tab(),
                NavigationAction::NextItem => self.select_next(),
                NavigationAction::PreviousItem => self.select_previous(),
            }
            return;
        }

        match key.code {
            KeyCode::Char('1') => self.set_tab(TabTarget::All),
            KeyCode::Char('2') => self.set_tab(TabTarget::Dev),
            KeyCode::Char('3') => self.set_tab(TabTarget::Tunnels),
            KeyCode::Char('4') => self.set_tab(TabTarget::System),
            KeyCode::PageDown => {
                self.detail_scroll = self
                    .detail_scroll
                    .saturating_add(self.regions.detail.height.saturating_sub(3).max(1))
            }
            KeyCode::PageUp => {
                self.detail_scroll = self
                    .detail_scroll
                    .saturating_sub(self.regions.detail.height.saturating_sub(3).max(1))
            }
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.select_last(),
            KeyCode::Char('r') => self.refresh(),
            KeyCode::Char('h') => self.prepare_hide(),
            KeyCode::Char('s') => self.prepare_contextual_stop(),
            KeyCode::Char(',') => self.open_settings(),
            KeyCode::Char('c') => self.open_text_selection(),
            _ => {}
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollDown if self.pending.is_none() && self.pending_hide.is_none() => {
                if self.settings_open {
                    self.select_next_setting();
                } else if rect_contains(self.regions.detail, mouse.column, mouse.row) {
                    self.detail_scroll = self.detail_scroll.saturating_add(3);
                } else {
                    self.select_next();
                }
            }
            MouseEventKind::ScrollUp if self.pending.is_none() && self.pending_hide.is_none() => {
                if self.settings_open {
                    self.select_previous_setting();
                } else if rect_contains(self.regions.detail, mouse.column, mouse.row) {
                    self.detail_scroll = self.detail_scroll.saturating_sub(3);
                } else {
                    self.select_previous();
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let visible_len = if self.settings_open {
                    self.config.display.hide.len()
                } else {
                    self.visible_group_indices().len()
                };
                let list_offset = if self.settings_open {
                    self.settings_list_offset
                } else {
                    self.list_offset
                };
                if let Some(action) = self.regions.action_at(
                    mouse,
                    list_offset,
                    visible_len,
                    self.pending.is_some(),
                    self.pending_hide.is_some(),
                    self.settings_open,
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
            MouseAction::SelectSetting(index) => self.settings_selected = index,
            MouseAction::Stop => self.prepare_contextual_stop(),
            MouseAction::Hide => self.prepare_hide(),
            MouseAction::OpenSettings => self.open_settings(),
            MouseAction::CloseSettings => {
                self.settings_open = false;
                self.status = self.tr.text("tui_settings_closed").into();
            }
            MouseAction::SelectText => self.open_text_selection(),
            MouseAction::Refresh => self.refresh(),
            MouseAction::Quit => self.should_quit = true,
            MouseAction::CopyCommand(index) => {
                let Some(command) = self
                    .regions
                    .command_copies
                    .get(index)
                    .map(|(_, command)| command.clone())
                else {
                    return;
                };
                self.status = match copy_to_terminal_clipboard(&command) {
                    Ok(()) => self.tr.text("tui_command_copied").into(),
                    Err(error) => self
                        .tr
                        .format("tui_command_copy_failed", &[("error", error.to_string())]),
                };
            }
            MouseAction::RemoveRule => self.remove_selected_rule(),
            MouseAction::Confirm => self.confirm_pending(),
            MouseAction::Cancel => {
                self.pending = None;
                self.pending_hide = None;
                self.status = self.tr.text("tui_cancelled").into();
            }
            MouseAction::ToggleHideField(index) => self.toggle_hide_field(index),
            MouseAction::SaveHide => self.save_pending_hide(),
        }
    }

    fn refresh(&mut self) {
        let selected_id = self.selected_group().map(|group| group.id.clone());
        (self.snapshot, self.hidden_count) = scan_visible(&self.config);
        self.last_refresh = Instant::now();
        self.status = scan_status(&self.snapshot, self.hidden_count, self.tr);
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
            self.status = self.tr.text("tui_no_actionable_resource").into();
            return;
        };
        match build_stop_plan(&[group], target, &Filters::default(), true) {
            Ok(services) => {
                self.pending = Some(PendingStop {
                    label: label.into(),
                    services,
                });
            }
            Err(error) => self.status = self.tr.plan_error(&error),
        }
    }

    fn prepare_contextual_stop(&mut self) {
        let target = stop_target_for_tab(self.tab);
        let label = self.tr.text(stop_label_key(self.tab));
        self.prepare_stop(target, label);
    }

    fn open_settings(&mut self) {
        self.settings_selected = self
            .settings_selected
            .min(self.config.display.hide.len().saturating_sub(1));
        self.settings_list_offset = 0;
        self.settings_open = true;
        self.status = self.tr.text("tui_settings_opened").into();
    }

    fn open_text_selection(&mut self) {
        self.text_selection_mode = true;
        self.status = self.tr.text("tui_text_selection_opened").into();
    }

    fn prepare_hide(&mut self) {
        if self.config_path.is_none() {
            self.status = self.tr.text("tui_hide_disabled").into();
            return;
        }
        let Some(group) = self.selected_group() else {
            self.status = self.tr.text("tui_no_actionable_resource").into();
            return;
        };
        let Some(service) = service_for_hide(group, self.tab).cloned() else {
            self.status = self.tr.text("tui_no_actionable_resource").into();
            return;
        };
        self.pending_hide = Some(PendingHide::new(service));
    }

    fn select_next_hide_field(&mut self) {
        if let Some(pending) = &mut self.pending_hide {
            pending.selected = (pending.selected + 1).min(pending.fields.len().saturating_sub(1));
        }
    }

    fn select_previous_hide_field(&mut self) {
        if let Some(pending) = &mut self.pending_hide {
            pending.selected = pending.selected.saturating_sub(1);
        }
    }

    fn toggle_selected_hide_field(&mut self) {
        if let Some(index) = self.pending_hide.as_ref().map(|pending| pending.selected) {
            self.toggle_hide_field(index);
        }
    }

    fn toggle_hide_field(&mut self, index: usize) {
        let Some(choice) = self
            .pending_hide
            .as_mut()
            .and_then(|pending| pending.fields.get_mut(index))
        else {
            return;
        };
        if choice.available {
            choice.checked = !choice.checked;
        }
    }

    fn save_pending_hide(&mut self) {
        let Some(path) = self.config_path.clone() else {
            self.status = self.tr.text("tui_hide_disabled").into();
            return;
        };
        let Some(pending) = self.pending_hide.take() else {
            return;
        };
        let fields = pending
            .fields
            .iter()
            .filter(|choice| choice.available && choice.checked)
            .map(|choice| choice.field)
            .collect::<Vec<_>>();
        let Some(rule) = HideRule::from_service(&pending.service, &fields) else {
            self.status = self.tr.text("tui_hide_no_fields").into();
            self.pending_hide = Some(pending);
            return;
        };

        self.config.display.hide.push(rule);
        if let Err(error) = save_config(&path, &self.config) {
            self.config.display.hide.pop();
            self.status = self.tr.format(
                "tui_hide_save_failed",
                &[("error", self.tr.config_error(&error))],
            );
            self.pending_hide = Some(pending);
            return;
        }
        self.refresh();
        self.status = self
            .tr
            .format("tui_hide_saved", &[("path", path.display().to_string())]);
    }

    fn remove_selected_rule(&mut self) {
        let Some(path) = self.config_path.clone() else {
            self.status = self.tr.text("tui_hide_disabled").into();
            return;
        };
        if self.config.display.hide.is_empty()
            || self.settings_selected >= self.config.display.hide.len()
        {
            self.status = self.tr.text("tui_no_rule_selected").into();
            return;
        }
        let removed = self.config.display.hide.remove(self.settings_selected);
        if let Err(error) = save_config(&path, &self.config) {
            self.config
                .display
                .hide
                .insert(self.settings_selected, removed);
            self.status = self.tr.format(
                "tui_remove_failed",
                &[("error", self.tr.config_error(&error))],
            );
            return;
        }
        self.settings_selected = self
            .settings_selected
            .min(self.config.display.hide.len().saturating_sub(1));
        self.refresh();
        self.status = self
            .tr
            .format("tui_remove_saved", &[("path", path.display().to_string())]);
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
                Err(error) => failures.push(format!(
                    "pid {}: {}",
                    service.pid,
                    self.tr.engine_error(&error)
                )),
            }
        }
        self.refresh();
        self.status = if failures.is_empty() {
            self.tr.stopped(stopped)
        } else {
            self.tr.format(
                "tui_stopped_with_failures",
                &[
                    ("stopped", stopped.to_string()),
                    ("failed", failures.len().to_string()),
                ],
            )
        };
    }

    fn next_tab(&mut self) {
        self.tab = next_tab_target(self.tab);
        self.selected = 0;
        self.list_offset = 0;
    }

    fn previous_tab(&mut self) {
        self.tab = previous_tab_target(self.tab);
        self.selected = 0;
        self.list_offset = 0;
    }

    fn set_tab(&mut self, tab: TabTarget) {
        self.tab = tab;
        self.selected = 0;
        self.list_offset = 0;
    }

    fn select_next(&mut self) {
        let len = self.selectable_len();
        if len > 0 {
            self.selected = (self.selected + 1).min(len - 1);
        }
    }

    fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn select_last(&mut self) {
        self.selected = self.selectable_len().saturating_sub(1);
    }

    fn selectable_len(&self) -> usize {
        self.visible_group_indices().len()
    }

    fn select_next_setting(&mut self) {
        if !self.config.display.hide.is_empty() {
            self.settings_selected =
                (self.settings_selected + 1).min(self.config.display.hide.len().saturating_sub(1));
        }
    }

    fn select_previous_setting(&mut self) {
        self.settings_selected = self.settings_selected.saturating_sub(1);
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
        frame.render_widget(Block::new().style(Style::default().bg(BASE).fg(TEXT)), area);
        let sections = Layout::vertical([
            Constraint::Length(TAB_HEIGHT),
            Constraint::Min(10),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .split(area);
        self.regions = UiRegions::new(
            area,
            self.tab,
            self.pending_hide
                .as_ref()
                .map_or(0, |pending| pending.fields.len()),
        );

        self.draw_tabs(frame, sections[0]);
        self.draw_content(frame, sections[1]);
        self.draw_footer(frame, sections[2]);
        if let Some(pending) = &self.pending {
            self.draw_confirmation(frame, area, pending);
        }
        if let Some(pending) = &self.pending_hide {
            self.draw_hide_editor(frame, area, pending);
        }
        if self.settings_open {
            self.draw_settings(frame);
        }
    }

    fn draw_tabs(&self, frame: &mut ratatui::Frame, area: Rect) {
        frame.render_widget(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(self.tr.text("tui_title"))
                .border_style(Style::default().fg(BORDER))
                .style(Style::default().fg(TEXT).bg(PANEL)),
            area,
        );
        let labels = [
            self.tr.text("tui_tab_all"),
            self.tr.text("tui_tab_dev"),
            self.tr.text("tui_tab_tunnels"),
            self.tr.text("tui_tab_system"),
        ];
        for ((tab_area, tab), label) in self.regions.tabs.iter().zip(labels) {
            let selected = *tab == self.tab;
            frame.render_widget(
                Paragraph::new(label)
                    .alignment(Alignment::Center)
                    .style(if selected {
                        Style::default()
                            .fg(Color::Black)
                            .bg(CYAN)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(MUTED).bg(PANEL)
                    }),
                *tab_area,
            );
        }
    }

    fn draw_content(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let columns = content_columns(area);
        let visible = self.visible_group_indices();
        let items = visible
            .iter()
            .map(|index| group_list_item(&self.snapshot.groups[*index], self.tr))
            .collect::<Vec<_>>();
        let list = List::new(items)
            .block(
                Block::bordered()
                    .title(panel_title(&self.tr.format(
                        "tui_resources_title",
                        &[("count", visible.len().to_string())],
                    )))
                    .border_style(Style::default().fg(BORDER))
                    .style(Style::default().fg(TEXT).bg(SURFACE))
                    .padding(MAIN_PANEL_PADDING),
            )
            .style(Style::default().fg(TEXT).bg(SURFACE))
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

        let group_id = self.selected_group().map(|group| group.id.clone());
        if self.detail_group_id != group_id {
            self.detail_scroll = 0;
            self.detail_group_id = group_id;
        }
        let detail = self
            .selected_group()
            .map(|group| group_detail(group, self.tr, columns[1].width.saturating_sub(4) as usize))
            .unwrap_or_else(|| DetailContent {
                text: Text::from(self.tr.text("tui_empty_tab")),
                commands: Vec::new(),
            });
        let content_height = columns[1].height.saturating_sub(2 + MAIN_PANEL_TOP_PADDING);
        let max_scroll = detail
            .text
            .lines
            .len()
            .saturating_sub(usize::from(content_height))
            .min(u16::MAX as usize) as u16;
        self.detail_scroll = self.detail_scroll.min(max_scroll);
        self.regions.command_copies = detail
            .commands
            .iter()
            .filter_map(|(row, command)| {
                let visible_row = row.checked_sub(self.detail_scroll)?;
                (visible_row < content_height)
                    .then(|| (command_copy_rect(columns[1], visible_row), command.clone()))
            })
            .collect();
        let detail = Paragraph::new(detail.text)
            .scroll((self.detail_scroll, 0))
            .style(Style::default().fg(TEXT).bg(SURFACE))
            .block(
                Block::bordered()
                    .title(panel_title(self.tr.text("tui_detail_title")))
                    .border_style(Style::default().fg(PURPLE))
                    .style(Style::default().fg(TEXT).bg(SURFACE))
                    .padding(MAIN_PANEL_PADDING),
            );
        frame.render_widget(detail, columns[1]);
    }

    fn draw_settings(&mut self, frame: &mut ratatui::Frame) {
        frame.render_widget(
            Block::new().style(Style::default().add_modifier(Modifier::DIM)),
            frame.area(),
        );
        frame.render_widget(Clear, self.regions.settings_popup);
        frame.render_widget(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(self.tr.text("tui_settings_title"))
                .title_alignment(Alignment::Center)
                .border_style(Style::default().fg(PURPLE))
                .style(Style::default().fg(TEXT).bg(PANEL)),
            self.regions.settings_popup,
        );
        let items = self
            .config
            .display
            .hide
            .iter()
            .enumerate()
            .map(|(index, rule)| rule_list_item(index, rule, self.tr))
            .collect::<Vec<_>>();
        let list = List::new(items)
            .block(
                Block::bordered()
                    .title(panel_title(&self.tr.format(
                        "tui_rules_title",
                        &[("count", self.config.display.hide.len().to_string())],
                    )))
                    .border_style(Style::default().fg(BORDER))
                    .style(Style::default().fg(TEXT).bg(SURFACE))
                    .padding(Padding::horizontal(1)),
            )
            .style(Style::default().fg(TEXT).bg(SURFACE))
            .highlight_symbol("▌ ")
            .highlight_style(
                Style::default()
                    .fg(Color::White)
                    .bg(SELECTED)
                    .add_modifier(Modifier::BOLD),
            );
        let has_rules = !self.config.display.hide.is_empty();
        let mut state = ListState::default()
            .with_selected(has_rules.then_some(self.settings_selected))
            .with_offset(self.settings_list_offset);
        frame.render_stateful_widget(list, self.regions.settings_list_panel, &mut state);
        self.settings_list_offset = state.offset();

        let detail = self
            .config
            .display
            .hide
            .get(self.settings_selected)
            .map(|rule| rule_detail(rule, self.tr, self.config_path.as_deref()))
            .unwrap_or_else(|| Text::from(self.tr.text("tui_empty_rules")));
        frame.render_widget(
            Paragraph::new(detail)
                .style(Style::default().fg(TEXT).bg(SURFACE))
                .block(
                    Block::bordered()
                        .title(panel_title(self.tr.text("tui_settings_detail_title")))
                        .border_style(Style::default().fg(PURPLE))
                        .style(Style::default().fg(TEXT).bg(SURFACE))
                        .padding(Padding::horizontal(1)),
                )
                .wrap(Wrap { trim: false }),
            self.regions.settings_detail_panel,
        );
        render_button(
            frame,
            self.regions.settings_remove,
            self.tr.text("tui_settings_remove_button"),
            DANGER,
        );
        render_button(
            frame,
            self.regions.settings_close,
            self.tr.text("tui_settings_close_button"),
            MUTED,
        );
    }

    fn draw_footer(&self, frame: &mut ratatui::Frame, area: Rect) {
        let action_row = footer_action_row(area);
        let actions = footer_actions(self.tab);
        let action_areas = footer_action_areas(action_row, actions.len());
        let compact = action_areas.first().is_some_and(|area| area.width < 15);
        for ((area, action), _) in action_areas.iter().zip(&actions).zip(0..) {
            let (label, key_label, danger) = action_label(*action, self.tab, self.tr, compact);
            let color = action_color(*action, danger);
            render_button(frame, *area, &format!("{key_label}  {label}"), color);
        }
        let status = Line::from(Span::styled(
            format!("  {}", self.status),
            Style::default().fg(if self.snapshot.warnings.is_empty() {
                MUTED
            } else {
                WARNING
            }),
        ));
        let status_area = Rect::new(
            area.x,
            area.y.saturating_add(FOOTER_ACTION_HEIGHT),
            area.width,
            1,
        );
        frame.render_widget(Paragraph::new(status), status_area);
    }

    fn draw_confirmation(&self, frame: &mut ratatui::Frame, area: Rect, pending: &PendingStop) {
        let popup = centered_rect(58, 9, area);
        frame.render_widget(Clear, popup);
        let text = Text::from(vec![
            Line::from(Span::styled(
                self.tr.format(
                    "tui_confirm_summary",
                    &[
                        ("label", pending.label.clone()),
                        ("count", pending.services.len().to_string()),
                    ],
                ),
                Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(self.tr.text("tui_confirm_hint")).alignment(Alignment::Center),
        ]);
        let paragraph = Paragraph::new(text).alignment(Alignment::Center).block(
            Block::bordered()
                .border_type(BorderType::Double)
                .border_style(Style::default().fg(DANGER))
                .title(self.tr.text("tui_confirm_title"))
                .style(Style::default().bg(PANEL)),
        );
        frame.render_widget(paragraph, popup);
        let [confirm, cancel] = confirmation_button_areas(popup);
        frame.render_widget(
            Paragraph::new(self.tr.text("tui_confirm_button"))
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::Black).bg(DANGER).bold()),
            confirm,
        );
        frame.render_widget(
            Paragraph::new(self.tr.text("tui_cancel_button"))
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::Black).bg(MUTED).bold()),
            cancel,
        );
    }

    fn draw_hide_editor(&self, frame: &mut ratatui::Frame, area: Rect, pending: &PendingHide) {
        let popup = centered_rect(72, 17, area);
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Block::bordered()
                .border_type(BorderType::Double)
                .border_style(Style::default().fg(PURPLE))
                .title(self.tr.text("tui_hide_title"))
                .style(Style::default().bg(PANEL)),
            popup,
        );
        let port = pending
            .service
            .tunnel_target
            .as_ref()
            .map(|target| target.port)
            .or_else(|| pending.service.ports.first().copied())
            .map_or_else(|| "-".into(), |port| port.to_string());
        frame.render_widget(
            Paragraph::new(self.tr.format(
                "tui_hide_summary",
                &[
                    ("runtime", runtime_slug(&pending.service.runtime).into()),
                    ("port", port),
                ],
            ))
            .alignment(Alignment::Center)
            .style(Style::default().fg(CYAN).bold()),
            Rect::new(popup.x + 2, popup.y + 2, popup.width.saturating_sub(4), 1),
        );
        for (index, (choice, field_area)) in pending
            .fields
            .iter()
            .zip(&self.regions.hide_fields)
            .enumerate()
        {
            let check = if choice.checked { "[x]" } else { "[ ]" };
            let label = hide_field_label(choice.field, self.tr);
            let value = if choice.available {
                choice.value.as_str()
            } else {
                self.tr.text("tui_value_unavailable")
            };
            let style = if !choice.available {
                Style::default().fg(Color::DarkGray)
            } else if index == pending.selected {
                Style::default().fg(Color::Black).bg(CYAN).bold()
            } else {
                Style::default().fg(Color::White)
            };
            frame.render_widget(
                Paragraph::new(format!(" {check} {label:<14} {value}")).style(style),
                *field_area,
            );
        }
        frame.render_widget(
            Paragraph::new(self.tr.text("tui_hide_hint"))
                .alignment(Alignment::Center)
                .style(Style::default().fg(MUTED)),
            Rect::new(
                popup.x + 2,
                popup.y + popup.height.saturating_sub(3),
                popup.width.saturating_sub(4),
                1,
            ),
        );
        frame.render_widget(
            Paragraph::new(self.tr.text("tui_confirm_button"))
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::Black).bg(PURPLE).bold()),
            self.regions.hide_save,
        );
        frame.render_widget(
            Paragraph::new(self.tr.text("tui_cancel_button"))
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::Black).bg(MUTED).bold()),
            self.regions.hide_cancel,
        );
    }
}

impl UiRegions {
    fn new(area: Rect, tab: TabTarget, hide_field_count: usize) -> Self {
        let [tabs_area, content_area, footer_area] = Layout::vertical([
            Constraint::Length(TAB_HEIGHT),
            Constraint::Min(10),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .areas(area);
        let [list_column, detail] = content_columns(content_area);
        let list = Rect::new(
            list_column.x.saturating_add(2),
            list_column.y.saturating_add(1 + MAIN_PANEL_TOP_PADDING),
            list_column.width.saturating_sub(4),
            list_column
                .height
                .saturating_sub(2 + MAIN_PANEL_TOP_PADDING),
        );
        let tab_row = Rect::new(
            tabs_area.x.saturating_add(PANEL_TITLE_LEFT_INSET),
            tabs_area.y.saturating_add(1),
            tabs_area
                .width
                .saturating_sub(PANEL_TITLE_LEFT_INSET.saturating_mul(2)),
            1,
        );
        let tab_areas = Layout::horizontal([Constraint::Ratio(1, 4); 4]).split(tab_row);
        let tabs = [
            TabTarget::All,
            TabTarget::Dev,
            TabTarget::Tunnels,
            TabTarget::System,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, tab)| (tab_areas[index], tab))
        .collect();
        let action_row = footer_action_row(footer_area);
        let actions = footer_actions(tab);
        let footer_areas = footer_action_areas(action_row, actions.len());
        let footer_actions = actions
            .into_iter()
            .enumerate()
            .map(|(index, action)| (footer_areas[index], action))
            .collect();
        let popup = centered_rect(58, 9, area);
        let [confirm, cancel] = confirmation_button_areas(popup);
        let hide_popup = centered_rect(72, 17, area);
        let hide_fields = (0..hide_field_count)
            .map(|index| {
                Rect::new(
                    hide_popup.x.saturating_add(3),
                    hide_popup.y.saturating_add(4 + index as u16),
                    hide_popup.width.saturating_sub(6),
                    1,
                )
            })
            .collect();
        let hide_save = Rect::new(
            hide_popup.x + hide_popup.width.saturating_sub(22),
            hide_popup.y + hide_popup.height.saturating_sub(2),
            19,
            1,
        );
        let hide_cancel = Rect::new(
            hide_popup.x.saturating_add(3),
            hide_popup.y + hide_popup.height.saturating_sub(2),
            19,
            1,
        );
        let settings_popup = settings_popup_rect(area);
        let settings_inner = Rect::new(
            settings_popup.x.saturating_add(2),
            settings_popup.y.saturating_add(2),
            settings_popup.width.saturating_sub(4),
            settings_popup.height.saturating_sub(4),
        );
        let [settings_content, settings_buttons] =
            Layout::vertical([Constraint::Min(6), Constraint::Length(1)]).areas(settings_inner);
        let [settings_list_panel, _, settings_detail_panel] = Layout::horizontal([
            Constraint::Percentage(43),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .areas(settings_content);
        let settings_list = Rect::new(
            settings_list_panel.x.saturating_add(2),
            settings_list_panel.y.saturating_add(1),
            settings_list_panel.width.saturating_sub(4),
            settings_list_panel.height.saturating_sub(2),
        );
        let [settings_remove, _, settings_close] = Layout::horizontal([
            Constraint::Length(22),
            Constraint::Length(2),
            Constraint::Length(22),
        ])
        .flex(Flex::Center)
        .areas(settings_buttons);
        Self {
            tabs,
            list,
            detail,
            footer_actions,
            confirm,
            cancel,
            hide_fields,
            hide_save,
            hide_cancel,
            settings_list,
            settings_list_panel,
            settings_detail_panel,
            settings_popup,
            settings_remove,
            settings_close,
            command_copies: Vec::new(),
        }
    }

    fn action_at(
        &self,
        mouse: MouseEvent,
        list_offset: usize,
        visible_len: usize,
        confirmation_open: bool,
        hide_open: bool,
        settings_open: bool,
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
        if hide_open {
            if let Some(index) = self
                .hide_fields
                .iter()
                .position(|area| rect_contains(*area, mouse.column, mouse.row))
            {
                return Some(MouseAction::ToggleHideField(index));
            }
            return rect_contains(self.hide_save, mouse.column, mouse.row)
                .then_some(MouseAction::SaveHide)
                .or_else(|| {
                    rect_contains(self.hide_cancel, mouse.column, mouse.row)
                        .then_some(MouseAction::Cancel)
                });
        }
        if settings_open {
            if rect_contains(self.settings_list, mouse.column, mouse.row) {
                let item = list_offset
                    + usize::from(
                        mouse.row.saturating_sub(self.settings_list.y) / RULE_ITEM_HEIGHT,
                    );
                return (item < visible_len).then_some(MouseAction::SelectSetting(item));
            }
            return rect_contains(self.settings_remove, mouse.column, mouse.row)
                .then_some(MouseAction::RemoveRule)
                .or_else(|| {
                    rect_contains(self.settings_close, mouse.column, mouse.row)
                        .then_some(MouseAction::CloseSettings)
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
            let item = list_offset
                + usize::from(mouse.row.saturating_sub(self.list.y) / GROUP_ITEM_HEIGHT);
            return (item < visible_len).then_some(MouseAction::Select(item));
        }
        if let Some((index, _)) = self
            .command_copies
            .iter()
            .enumerate()
            .find(|(_, (area, _))| rect_contains(*area, mouse.column, mouse.row))
        {
            return Some(MouseAction::CopyCommand(index));
        }
        self.footer_actions
            .iter()
            .find(|(area, _)| rect_contains(*area, mouse.column, mouse.row))
            .map(|(_, action)| *action)
    }
}

fn footer_actions(_tab: TabTarget) -> Vec<MouseAction> {
    vec![
        MouseAction::Hide,
        MouseAction::Stop,
        MouseAction::OpenSettings,
        MouseAction::SelectText,
        MouseAction::Refresh,
        MouseAction::Quit,
    ]
}

fn footer_action_row(area: Rect) -> Rect {
    Rect::new(area.x, area.y, area.width, FOOTER_ACTION_HEIGHT)
}

fn footer_action_areas(area: Rect, count: usize) -> Vec<Rect> {
    let width = footer_button_width(area, count);
    let mut constraints = Vec::with_capacity(count.saturating_mul(2));
    for index in 0..count {
        constraints.push(Constraint::Length(width));
        if index + 1 < count {
            constraints.push(Constraint::Length(1));
        }
    }
    constraints.push(Constraint::Min(0));
    Layout::horizontal(constraints)
        .split(area)
        .iter()
        .step_by(2)
        .take(count)
        .copied()
        .collect()
}

fn footer_button_width(area: Rect, count: usize) -> u16 {
    if count == 0 {
        return 0;
    }
    let gaps = count.saturating_sub(1) as u16;
    let available = area.width.saturating_sub(gaps);
    let full_width = (count as u16).saturating_mul(20).saturating_add(gaps);
    if area.width >= full_width {
        20
    } else {
        (available / count as u16).min(20)
    }
}

fn action_color(action: MouseAction, danger: bool) -> Color {
    if danger {
        DANGER
    } else if action == MouseAction::OpenSettings {
        PURPLE
    } else if action == MouseAction::Quit {
        MUTED
    } else {
        CYAN
    }
}

fn action_label(
    action: MouseAction,
    tab: TabTarget,
    tr: Translator,
    compact: bool,
) -> (&'static str, &'static str, bool) {
    match action {
        MouseAction::Hide => (tr.text("tui_action_hide"), "h", false),
        MouseAction::RemoveRule => (tr.text("tui_action_remove"), "u", true),
        MouseAction::Stop => (tr.text(stop_action_label_key(tab, compact)), "s", true),
        MouseAction::OpenSettings => (tr.text("tui_action_settings"), ",", false),
        MouseAction::SelectText => (
            tr.text(if compact {
                "tui_action_select_text_short"
            } else {
                "tui_action_select_text"
            }),
            "c",
            false,
        ),
        MouseAction::Refresh => (tr.text("tui_action_refresh"), "r", false),
        MouseAction::Quit => (tr.text("tui_action_quit"), "q", false),
        _ => ("", "", false),
    }
}

fn render_button(frame: &mut ratatui::Frame, area: Rect, label: &str, background: Color) {
    frame.render_widget(
        Block::new().style(Style::default().fg(BASE).bg(background).bold()),
        area,
    );
    let label_row = Rect::new(
        area.x,
        area.y.saturating_add(area.height.saturating_sub(1) / 2),
        area.width,
        1,
    );
    frame.render_widget(
        Paragraph::new(label)
            .alignment(Alignment::Center)
            .style(Style::default().fg(BASE).bg(background).bold()),
        label_row,
    );
}

fn panel_title(title: &str) -> Line<'static> {
    Line::from(format!("  {}  ", title.trim()))
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

fn command_copy_rect(panel: Rect, row: u16) -> Rect {
    Rect::new(
        panel.x.saturating_add(2),
        panel
            .y
            .saturating_add(row.saturating_add(1 + MAIN_PANEL_TOP_PADDING)),
        panel.width.saturating_sub(4),
        1,
    )
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

impl PendingHide {
    fn new(service: ServiceProcess) -> Self {
        let port_value = service
            .ports
            .iter()
            .copied()
            .chain(service.tunnel_target.iter().map(|target| target.port))
            .map(|port| port.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let project = service
            .project_name
            .as_ref()
            .or(service.cwd.as_ref())
            .cloned()
            .unwrap_or_default();
        let fields = vec![
            hide_choice(HideField::Port, port_value, true),
            hide_choice(
                HideField::Runtime,
                runtime_slug(&service.runtime).into(),
                true,
            ),
            hide_choice(
                HideField::Kind,
                resource_kind_slug(&service.resource_kind).into(),
                false,
            ),
            hide_choice(HideField::Platform, service.platform.as_str().into(), false),
            hide_choice(HideField::ProcessName, service.process_name.clone(), false),
            hide_choice(HideField::Project, project, false),
            hide_choice(HideField::Command, service.command.clone(), false),
            hide_choice(HideField::Id, service.id.clone(), false),
        ];
        Self {
            service,
            fields,
            selected: 0,
        }
    }
}

fn hide_choice(field: HideField, value: String, checked: bool) -> HideChoice {
    HideChoice {
        field,
        available: !value.is_empty(),
        value,
        checked,
    }
}

fn service_for_hide(group: &ResourceGroup, tab: TabTarget) -> Option<&ServiceProcess> {
    let kind = match tab {
        TabTarget::Dev => Some(ResourceKind::Development),
        TabTarget::Tunnels => Some(ResourceKind::Tunnel),
        TabTarget::System => Some(ResourceKind::System),
        TabTarget::All => None,
    };
    kind.and_then(|kind| {
        group
            .services
            .iter()
            .find(|service| service.resource_kind == kind)
    })
    .or_else(|| {
        group
            .services
            .iter()
            .find(|service| service.resource_kind != ResourceKind::Tunnel)
    })
    .or_else(|| group.services.first())
}

fn hide_field_label(field: HideField, tr: Translator) -> &'static str {
    match field {
        HideField::Port => tr.text("tui_field_port"),
        HideField::Runtime => tr.text("tui_field_runtime"),
        HideField::Kind => tr.text("tui_field_kind"),
        HideField::Platform => tr.text("tui_field_platform"),
        HideField::ProcessName => tr.text("tui_field_process_name"),
        HideField::Project => tr.text("tui_field_project"),
        HideField::Command => tr.text("tui_field_command"),
        HideField::Id => tr.text("tui_field_id"),
    }
}

fn resource_kind_slug(kind: &ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Development => "dev",
        ResourceKind::Tunnel => "tunnel",
        ResourceKind::System => "system",
        ResourceKind::Other => "other",
    }
}

fn rule_list_item(index: usize, rule: &HideRule, tr: Translator) -> ListItem<'static> {
    let parts = rule_parts(rule, tr);
    ListItem::new(vec![
        Line::from(Span::styled(
            format!(
                "#{}  {}",
                index + 1,
                parts.first().cloned().unwrap_or_default()
            ),
            Style::default().fg(Color::White),
        )),
        Line::from(Span::styled(
            parts.into_iter().skip(1).collect::<Vec<_>>().join(" · "),
            Style::default().fg(MUTED),
        )),
        Line::from(""),
    ])
    .style(Style::default().fg(TEXT).bg(SURFACE))
}

fn rule_detail(rule: &HideRule, tr: Translator, path: Option<&std::path::Path>) -> Text<'static> {
    let path = path.map_or_else(
        || tr.text("tui_config_read_only").into(),
        |path| tr.format("tui_config_path", &[("path", path.display().to_string())]),
    );
    let mut lines = vec![
        Line::from(Span::styled(path, Style::default().fg(CYAN))),
        Line::from(""),
        Line::from(tr.text("tui_rule_matches")),
        Line::from(""),
    ];
    lines.extend(rule_parts(rule, tr).into_iter().map(Line::from));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        tr.text("tui_rule_remove_hint"),
        Style::default().fg(MUTED),
    )));
    Text::from(lines)
}

fn rule_parts(rule: &HideRule, tr: Translator) -> Vec<String> {
    let mut parts = Vec::new();
    push_rule_part(
        &mut parts,
        tr.text("tui_field_port"),
        rule.ports.iter().map(u16::to_string).collect(),
    );
    push_rule_part(
        &mut parts,
        tr.text("tui_field_runtime"),
        rule.runtimes.clone(),
    );
    push_rule_part(
        &mut parts,
        tr.text("tui_field_kind"),
        rule.kinds
            .iter()
            .map(|kind| match kind {
                ConfigResourceKind::Dev => "dev",
                ConfigResourceKind::Tunnel => "tunnel",
                ConfigResourceKind::System => "system",
                ConfigResourceKind::Other => "other",
            })
            .map(str::to_owned)
            .collect(),
    );
    push_rule_part(
        &mut parts,
        tr.text("tui_field_platform"),
        rule.platforms
            .iter()
            .map(|platform| match platform {
                ConfigPlatform::Windows => "windows",
                ConfigPlatform::Linux => "linux",
                ConfigPlatform::Macos => "macos",
            })
            .map(str::to_owned)
            .collect(),
    );
    push_rule_part(
        &mut parts,
        tr.text("tui_field_process_name"),
        rule.process_names.clone(),
    );
    push_rule_part(
        &mut parts,
        tr.text("tui_field_project"),
        rule.projects.clone(),
    );
    push_rule_part(
        &mut parts,
        tr.text("tui_field_command"),
        rule.commands.clone(),
    );
    push_rule_part(&mut parts, tr.text("tui_field_id"), rule.ids.clone());
    parts
}

fn push_rule_part(parts: &mut Vec<String>, label: &str, values: Vec<String>) {
    if !values.is_empty() {
        parts.push(format!("{label}: {}", values.join(", ")));
    }
}

fn group_list_item(group: &ResourceGroup, tr: Translator) -> ListItem<'static> {
    let primary = group
        .services
        .iter()
        .find(|service| service.resource_kind != ResourceKind::Tunnel)
        .or_else(|| group.services.first());
    let name = primary
        .map(|service| service.display_name.as_str())
        .unwrap_or(tr.text("tui_unknown"))
        .to_owned();
    let port = group
        .primary_port
        .map(|port| format!(":{port}"))
        .unwrap_or_else(|| tr.text("tui_no_port").into());
    let tunnels = group
        .services
        .iter()
        .filter(|service| service.resource_kind == ResourceKind::Tunnel)
        .count();
    let suffix = if tunnels > 0 {
        tr.format(
            "tui_items_tunnel",
            &[
                ("count", group.services.len().to_string()),
                ("tunnels", tunnels.to_string()),
            ],
        )
    } else {
        tr.format("tui_item", &[("count", group.services.len().to_string())])
    };
    ListItem::new(vec![
        Line::from(vec![
            Span::styled(port, Style::default().fg(CYAN).bold()),
            Span::raw("  "),
            Span::styled(name, Style::default().fg(TEXT)),
        ]),
        Line::from(Span::styled(suffix, Style::default().fg(MUTED))),
        Line::from(""),
    ])
    .style(Style::default().fg(TEXT).bg(SURFACE))
}

fn group_detail(group: &ResourceGroup, tr: Translator, width: usize) -> DetailContent {
    let mut lines = Vec::new();
    let mut commands = Vec::new();
    let now = unix_now();
    lines.push(route_line(group, tr, width));
    lines.push(Line::from(""));
    for (index, service) in group.services.iter().enumerate() {
        let role = match service.resource_kind {
            ResourceKind::Development => tr.text("tui_role_service"),
            ResourceKind::Tunnel => tr.text("tui_role_tunnel"),
            ResourceKind::System => tr.text("tui_role_system"),
            ResourceKind::Other => tr.text("tui_role_process"),
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
                service.display_name.clone(),
                Style::default().fg(Color::White).bold(),
            ),
            Span::styled(
                format!("  ({})", runtime_slug(&service.runtime)),
                Style::default().fg(MUTED),
            ),
            Span::styled(format!("  PID {}", service.pid), Style::default().fg(MUTED)),
        ]));
        lines.push(detail_line(
            tr.text("tui_field_scope"),
            scope_label(service),
            width,
        ));
        lines.push(detail_line(
            tr.text("process_name_short"),
            service.process_name.clone(),
            width,
        ));
        lines.push(detail_line(
            tr.text("process_started_at"),
            format_started_at(service.started_at, tr),
            width,
        ));
        lines.push(detail_line(
            tr.text("process_uptime"),
            format_uptime(service.started_at, now, tr),
            width,
        ));
        if !service.ports.is_empty() {
            lines.push(detail_line(
                tr.text("tui_field_listen"),
                service
                    .ports
                    .iter()
                    .map(|port| format!(":{port}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                width,
            ));
        }
        if let Some(target) = &service.tunnel_target {
            lines.push(detail_line(
                tr.text("tui_field_proxy"),
                format!("{}:{}", target.host, target.port),
                width,
            ));
        }
        if let Some(cwd) = &service.cwd {
            lines.push(detail_line(tr.text("tui_field_cwd"), cwd.clone(), width));
        }
        if let Some(unit) = &service.manager_unit {
            lines.push(detail_line(tr.text("tui_field_unit"), unit.clone(), width));
        }
        let wrapped_command = wrap_command(&service.command, width.saturating_sub(9).max(1));
        for (command_line_index, command_line) in wrapped_command.into_iter().enumerate() {
            let label = if command_line_index == 0 {
                padded_label(tr.text("tui_field_command"), 9)
            } else {
                " ".repeat(9)
            };
            lines.push(Line::from(vec![
                Span::styled(label, Style::default().fg(MUTED)),
                Span::raw(command_line),
            ]));
        }
        lines.push(Line::from(""));
        let copy_row = lines.len() as u16;
        lines.push(solid_button_line(
            tr.text("tui_copy_command_button"),
            width,
            CYAN,
        ));
        commands.push((copy_row, service.command.clone()));
        if !service.can_terminate {
            lines.push(Line::from(Span::styled(
                tr.text("tui_protected"),
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
    DetailContent {
        text: Text::from(lines),
        commands,
    }
}

fn detail_line(label: &str, value: String, width: usize) -> Line<'static> {
    let value = truncate_text(&value, width.saturating_sub(9));
    Line::from(vec![
        Span::styled(padded_label(label, 9), Style::default().fg(MUTED)),
        Span::raw(value),
    ])
}

fn padded_label(value: &str, width: usize) -> String {
    format!(
        "{value}{}",
        " ".repeat(width.saturating_sub(UnicodeWidthStr::width(value)))
    )
}

fn solid_button_line(label: &str, width: usize, background: Color) -> Line<'static> {
    let label_width = UnicodeWidthStr::width(label);
    let available = width.saturating_sub(label_width);
    let left = available / 2;
    let right = available.saturating_sub(left);
    Line::from(Span::styled(
        format!("{}{}{}", " ".repeat(left), label, " ".repeat(right)),
        Style::default().fg(BASE).bg(background).bold(),
    ))
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    if max_chars == 1 {
        return "…".into();
    }
    let mut truncated = value.chars().take(max_chars - 1).collect::<String>();
    truncated.push('…');
    truncated
}

fn wrap_command(value: &str, max_chars: usize) -> Vec<String> {
    if value.is_empty() || max_chars == 0 {
        return vec![value.to_owned()];
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width: usize = 0;
    for character in value.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if !line.is_empty() && line_width.saturating_add(character_width) > max_chars {
            lines.push(std::mem::take(&mut line));
            line_width = 0;
        }
        line.push(character);
        line_width = line_width.saturating_add(character_width);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn osc52_sequence(value: &str) -> String {
    format!("\u{1b}]52;c;{}\u{7}", BASE64.encode(value.as_bytes()))
}

fn copy_to_terminal_clipboard(value: &str) -> io::Result<()> {
    let mut stdout = io::stdout();
    stdout.write_all(osc52_sequence(value).as_bytes())?;
    stdout.flush()
}

fn route_line(group: &ResourceGroup, tr: Translator, width: usize) -> Line<'static> {
    let tunnel = group
        .services
        .iter()
        .find(|service| service.resource_kind == ResourceKind::Tunnel);
    let source = group
        .services
        .iter()
        .find(|service| service.resource_kind != ResourceKind::Tunnel);
    match (tunnel, source, group.primary_port) {
        (Some(tunnel), Some(source), Some(port)) => {
            let public = tr.text("tui_route_public");
            let runtime = runtime_slug(&tunnel.runtime);
            let fixed_width = public.chars().count() + runtime.chars().count() + 14;
            let source = truncate_text(
                &format!(":{port} {}", source.display_name.as_str()),
                width.saturating_sub(fixed_width),
            );
            Line::from(vec![
                Span::styled(public, Style::default().fg(PURPLE).bold()),
                Span::styled("  ──▶  ", Style::default().fg(MUTED)),
                Span::styled(runtime, Style::default().fg(PURPLE)),
                Span::styled("  ──▶  ", Style::default().fg(MUTED)),
                Span::styled(source, Style::default().fg(CYAN).bold()),
            ])
        }
        (_, Some(source), Some(port)) => Line::from(Span::styled(
            truncate_text(
                &format!(
                    "{} :{port} / {}",
                    tr.text("tui_route_local"),
                    source.display_name.as_str()
                ),
                width,
            ),
            Style::default().fg(CYAN).bold(),
        )),
        (Some(tunnel), _, Some(port)) => Line::from(Span::styled(
            truncate_text(
                &format!("{} ──▶ localhost:{port}", runtime_slug(&tunnel.runtime)),
                width,
            ),
            Style::default().fg(PURPLE).bold(),
        )),
        _ => Line::from(Span::styled(
            tr.text("tui_route_unbound"),
            Style::default().fg(MUTED),
        )),
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

fn scan_status(snapshot: &ScanSnapshot, hidden_count: usize, tr: Translator) -> String {
    let resources = snapshot
        .groups
        .iter()
        .map(|group| group.services.len())
        .sum::<usize>();
    tr.scan_status(
        snapshot.groups.len(),
        resources,
        snapshot.warnings.len(),
        hidden_count,
    )
}

fn content_columns(area: Rect) -> [Rect; 2] {
    let [list, _, detail] = Layout::horizontal([
        Constraint::Percentage(43),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(area);
    [list, detail]
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

fn settings_popup_rect(area: Rect) -> Rect {
    centered_rect(84, area.height.saturating_sub(6).clamp(10, 20), area)
}

fn navigation_action(key: KeyCode) -> Option<NavigationAction> {
    match key {
        KeyCode::Right | KeyCode::Tab => Some(NavigationAction::NextTab),
        KeyCode::Left | KeyCode::BackTab => Some(NavigationAction::PreviousTab),
        KeyCode::Down | KeyCode::Char('j') => Some(NavigationAction::NextItem),
        KeyCode::Up | KeyCode::Char('k') => Some(NavigationAction::PreviousItem),
        _ => None,
    }
}

fn next_tab_target(tab: TabTarget) -> TabTarget {
    match tab {
        TabTarget::All => TabTarget::Dev,
        TabTarget::Dev => TabTarget::Tunnels,
        TabTarget::Tunnels => TabTarget::System,
        TabTarget::System => TabTarget::All,
    }
}

fn previous_tab_target(tab: TabTarget) -> TabTarget {
    match tab {
        TabTarget::All => TabTarget::System,
        TabTarget::Dev => TabTarget::All,
        TabTarget::Tunnels => TabTarget::Dev,
        TabTarget::System => TabTarget::Tunnels,
    }
}

fn stop_target_for_tab(tab: TabTarget) -> StopTarget {
    match tab {
        TabTarget::All => StopTarget::Group,
        TabTarget::Dev => StopTarget::Dev,
        TabTarget::Tunnels => StopTarget::Tunnel,
        TabTarget::System => StopTarget::System,
    }
}

fn stop_label_key(tab: TabTarget) -> &'static str {
    match tab {
        TabTarget::All => "tui_stop_group",
        TabTarget::Dev => "tui_stop_dev",
        TabTarget::Tunnels => "tui_stop_tunnel",
        TabTarget::System => "tui_stop_system",
    }
}

fn stop_action_label_key(tab: TabTarget, compact: bool) -> &'static str {
    match (tab, compact) {
        (TabTarget::All, true) => "tui_action_stop_group_short",
        (TabTarget::Dev, true) => "tui_action_stop_dev_short",
        (TabTarget::Tunnels, true) => "tui_action_stop_tunnel_short",
        (TabTarget::System, true) => "tui_action_stop_service_short",
        (TabTarget::All, false) => "tui_action_stop_group",
        (TabTarget::Dev, false) => "tui_action_stop_dev",
        (TabTarget::Tunnels, false) => "tui_action_stop_tunnel",
        (TabTarget::System, false) => "tui_action_stop_service",
    }
}

fn text_selection_transition(active: bool, key: KeyCode) -> Option<bool> {
    match (active, key) {
        (false, KeyCode::Char('c')) => Some(true),
        (true, KeyCode::Char('c') | KeyCode::Esc) => Some(false),
        _ => None,
    }
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

    use super::{
        MouseAction, NavigationAction, UiRegions, command_copy_rect, footer_actions, is_quit_key,
        navigation_action, next_tab_target, osc52_sequence, padded_label, previous_tab_target,
        stop_target_for_tab, text_selection_transition, truncate_text, wrap_command,
    };
    use port_deck_cli::{StopTarget, TabTarget};

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
    fn arrow_keys_navigate_tabs_and_content() {
        assert_eq!(
            navigation_action(KeyCode::Right),
            Some(NavigationAction::NextTab)
        );
        assert_eq!(
            navigation_action(KeyCode::Left),
            Some(NavigationAction::PreviousTab)
        );
        assert_eq!(
            navigation_action(KeyCode::Down),
            Some(NavigationAction::NextItem)
        );
        assert_eq!(
            navigation_action(KeyCode::Up),
            Some(NavigationAction::PreviousItem)
        );
        assert_eq!(next_tab_target(TabTarget::System), TabTarget::All);
        assert_eq!(previous_tab_target(TabTarget::All), TabTarget::System);
    }

    #[test]
    fn text_selection_mode_has_an_explicit_keyboard_toggle() {
        assert_eq!(
            text_selection_transition(false, KeyCode::Char('c')),
            Some(true)
        );
        assert_eq!(
            text_selection_transition(true, KeyCode::Char('c')),
            Some(false)
        );
        assert_eq!(text_selection_transition(true, KeyCode::Esc), Some(false));
        assert_eq!(text_selection_transition(false, KeyCode::Esc), None);
    }

    #[test]
    fn each_resource_tab_exposes_one_contextual_stop_action() {
        assert_eq!(stop_target_for_tab(TabTarget::All), StopTarget::Group);
        assert_eq!(stop_target_for_tab(TabTarget::Dev), StopTarget::Dev);
        assert_eq!(stop_target_for_tab(TabTarget::Tunnels), StopTarget::Tunnel);
        assert_eq!(stop_target_for_tab(TabTarget::System), StopTarget::System);
        assert_eq!(
            footer_actions(TabTarget::All),
            vec![
                MouseAction::Hide,
                MouseAction::Stop,
                MouseAction::OpenSettings,
                MouseAction::SelectText,
                MouseAction::Refresh,
                MouseAction::Quit,
            ]
        );
    }

    #[test]
    fn mouse_clicks_map_to_tabs_rows_and_footer_actions() {
        let regions = UiRegions::new(Rect::new(0, 0, 100, 30), TabTarget::All, 0);

        assert_eq!(
            regions.action_at(mouse_down(3, 1), 0, 8, false, false, false),
            Some(MouseAction::SetTab(TabTarget::All))
        );
        assert_eq!(
            regions.action_at(mouse_down(50, 1), 0, 8, false, false, false),
            Some(MouseAction::SetTab(TabTarget::Tunnels))
        );
        assert_eq!(
            regions.action_at(mouse_down(8, 8), 2, 8, false, false, false),
            Some(MouseAction::Select(3))
        );
        assert_eq!(
            regions.action_at(mouse_down(30, 26), 0, 8, false, false, false),
            Some(MouseAction::Select(7))
        );
        assert_eq!(
            regions.action_at(mouse_down(30, 28), 0, 8, false, false, false),
            Some(MouseAction::Stop)
        );
        assert_eq!(
            regions.action_at(mouse_down(0, 28), 0, 8, false, false, false),
            Some(MouseAction::Hide)
        );
        assert_eq!(
            regions.action_at(mouse_down(55, 28), 0, 8, false, false, false),
            Some(MouseAction::SelectText)
        );
        assert_eq!(
            regions.action_at(mouse_down(75, 28), 0, 8, false, false, false),
            Some(MouseAction::Refresh)
        );
        assert_eq!(
            regions.action_at(mouse_down(90, 28), 0, 8, false, false, false),
            Some(MouseAction::Quit)
        );
        assert_eq!(
            regions.action_at(mouse_down(90, 29), 0, 8, false, false, false),
            None
        );
    }

    #[test]
    fn long_detail_values_are_ellipsized_to_the_available_width() {
        assert_eq!(
            truncate_text("cloudflared tunnel --url localhost:5000", 18),
            "cloudflared tunne…"
        );
        assert_eq!(truncate_text("vite", 18), "vite");
        assert_eq!(truncate_text("vite", 0), "");
    }

    #[test]
    fn command_wrapping_preserves_the_complete_command() {
        assert_eq!(
            wrap_command("cloudflared tunnel --url localhost:5000", 12),
            vec!["cloudflared ", "tunnel --url", " localhost:5", "000"]
        );
        assert_eq!(wrap_command("echo 你好", 6), vec!["echo ", "你好"]);
        assert_eq!(wrap_command("", 12), vec![""]);
    }

    #[test]
    fn detail_labels_use_terminal_cell_width_for_cjk_text() {
        assert_eq!(padded_label("命令", 9), "命令     ");
        assert_eq!(padded_label("command", 9), "command  ");
    }

    #[test]
    fn clipboard_sequence_uses_the_terminal_osc52_protocol() {
        assert_eq!(
            osc52_sequence("npm run dev"),
            "\u{1b}]52;c;bnBtIHJ1biBkZXY=\u{7}"
        );
    }

    #[test]
    fn command_copy_button_spans_the_detail_content_width() {
        let mut regions = UiRegions::new(Rect::new(0, 0, 100, 30), TabTarget::All, 0);
        regions.command_copies = vec![(
            command_copy_rect(Rect::new(44, 4, 56, 24), 15),
            "npm run dev".into(),
        )];

        assert_eq!(
            regions.action_at(mouse_down(46, 21), 0, 0, false, false, false),
            Some(MouseAction::CopyCommand(0))
        );
        assert_eq!(
            regions.action_at(mouse_down(97, 21), 0, 0, false, false, false),
            Some(MouseAction::CopyCommand(0))
        );
    }

    #[test]
    fn the_entire_equal_width_tab_area_is_clickable() {
        let regions = UiRegions::new(Rect::new(0, 0, 100, 30), TabTarget::All, 0);

        assert_eq!(
            regions.action_at(mouse_down(18, 1), 0, 8, false, false, false),
            Some(MouseAction::SetTab(TabTarget::All))
        );
        assert_eq!(
            regions.action_at(mouse_down(38, 1), 0, 8, false, false, false),
            Some(MouseAction::SetTab(TabTarget::Dev))
        );
        assert_eq!(
            regions.action_at(mouse_down(58, 1), 0, 8, false, false, false),
            Some(MouseAction::SetTab(TabTarget::Tunnels))
        );
        assert_eq!(
            regions.action_at(mouse_down(78, 1), 0, 8, false, false, false),
            Some(MouseAction::SetTab(TabTarget::System))
        );
        assert_eq!(
            regions.action_at(mouse_down(96, 1), 0, 8, false, false, false),
            Some(MouseAction::SetTab(TabTarget::System))
        );
    }

    #[test]
    fn confirmation_dialog_captures_mouse_clicks() {
        let regions = UiRegions::new(Rect::new(0, 0, 100, 30), TabTarget::All, 0);

        assert_eq!(
            regions.action_at(mouse_down(38, 17), 0, 8, true, false, false),
            Some(MouseAction::Confirm)
        );
        assert_eq!(
            regions.action_at(mouse_down(57, 17), 0, 8, true, false, false),
            Some(MouseAction::Cancel)
        );
        assert_eq!(
            regions.action_at(mouse_down(3, 1), 0, 8, true, false, false),
            None
        );
    }

    #[test]
    fn hide_rule_editor_fields_and_save_button_are_clickable() {
        let regions = UiRegions::new(Rect::new(0, 0, 100, 30), TabTarget::All, 8);

        assert_eq!(
            regions.action_at(mouse_down(20, 11), 0, 8, false, true, false),
            Some(MouseAction::ToggleHideField(0))
        );
        assert_eq!(
            regions.action_at(mouse_down(70, 22), 0, 8, false, true, false),
            Some(MouseAction::SaveHide)
        );
        assert_eq!(
            regions.action_at(mouse_down(20, 22), 0, 8, false, true, false),
            Some(MouseAction::Cancel)
        );
    }

    #[test]
    fn settings_dialog_rules_and_buttons_are_clickable() {
        let regions = UiRegions::new(Rect::new(0, 0, 100, 30), TabTarget::All, 0);

        assert_eq!(
            regions.action_at(mouse_down(15, 10), 0, 3, false, false, true),
            Some(MouseAction::SelectSetting(0))
        );
        assert_eq!(
            regions.action_at(mouse_down(15, 11), 0, 3, false, false, true),
            Some(MouseAction::SelectSetting(1))
        );
        assert_eq!(
            regions.action_at(mouse_down(30, 20), 0, 3, false, false, true),
            None
        );
        assert_eq!(
            regions.action_at(mouse_down(30, 22), 0, 3, false, false, true),
            Some(MouseAction::RemoveRule)
        );
        assert_eq!(
            regions.action_at(mouse_down(55, 22), 0, 3, false, false, true),
            Some(MouseAction::CloseSettings)
        );
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
