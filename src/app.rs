use crate::ascii_images;
use crossterm::event;
use ratatui::{layout, style::Stylize, symbols, text, widgets, DefaultTerminal, Frame};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time;

const DURATION_PRESETS: [u64; 11] = [2, 3, 4, 5, 10, 15, 20, 25, 30, 45, 60];

enum Event {
    Key(event::KeyEvent),
    Tick,
}

#[derive(Debug, PartialEq)]
enum MenuState {
    None,
    MainMenu,
    SelectWorkDuration,
    SelectBreakDuration,
    ExtendWorkSession,
    SelectSound,
}

pub struct App {
    pomo: pomodoro_tui::Pomodoro,
    exit: bool,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
    hide_image: bool,
    menu_state: MenuState,
    menu_selection: usize,
}

impl App {
    pub fn new(
        work_min: u64,
        break_min: u64,
        hide_image: bool,
        sound: &Path,
        no_sound: bool,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        App {
            pomo: pomodoro_tui::Pomodoro::new(
                (work_min, 0),
                (break_min, 0),
                sound.to_path_buf(),
                no_sound,
            ),
            exit: false,
            tx,
            rx,
            hide_image,
            menu_state: MenuState::None,
            menu_selection: 0,
        }
    }

    pub fn run(&mut self, mut terminal: DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            match self.rx.recv() {
                Ok(Event::Key(key_event)) => self.handle_key_event(key_event),
                Ok(Event::Tick) => {
                    let work_session_ended = self.pomo.check_and_switch();
                    if work_session_ended {
                        self.menu_state = MenuState::ExtendWorkSession;
                        self.menu_selection = 0;
                    }
                }
                _ => (),
            }
        }
        Ok(())
    }

    pub fn handle_inputs(&self) {
        let tx = self.tx.clone();
        let tick_rate = time::Duration::from_millis(200);
        std::thread::spawn(move || {
            let mut last_tick = time::Instant::now();
            loop {
                let timeout = tick_rate.saturating_sub(last_tick.elapsed());
                if event::poll(timeout).unwrap() {
                    match event::read().unwrap() {
                        event::Event::Key(key_event) => tx.send(Event::Key(key_event)).unwrap(),
                        _ => (),
                    }
                }
                if last_tick.elapsed() >= tick_rate {
                    tx.send(Event::Tick).unwrap();
                    last_tick = time::Instant::now();
                }
            }
        });
    }

    pub fn start_or_pause(&mut self) {
        self.pomo.start_or_pause();
    }

    fn draw(&self, frame: &mut Frame) {
        let (work_size, work_pixel, break_size, break_pixel) = match self.pomo.state() {
            pomodoro_tui::PomodoroState::Work => (
                8,
                tui_big_text::PixelSize::Full,
                4,
                tui_big_text::PixelSize::Quadrant,
            ),
            pomodoro_tui::PomodoroState::Break => (
                4,
                tui_big_text::PixelSize::Quadrant,
                8,
                tui_big_text::PixelSize::Full,
            ),
        };

        let area = frame.area();

        let block = self.get_block_widget();
        frame.render_widget(block, area);

        let (lcenter, rtop, rbottom) = self.get_layout(area, work_size, break_size);

        if !self.hide_image {
            let ascii_img = self.get_ascii_image_widget();
            frame.render_widget(ascii_img, lcenter);
        }

        let (work_timer, break_timer) = self.get_timer_widgets(work_pixel, break_pixel);
        frame.render_widget(work_timer, rtop);
        frame.render_widget(break_timer, rbottom);

        // Render menu overlay if menu is active
        if self.menu_state != MenuState::None {
            self.render_menu(frame, area);
        }
    }

    fn get_layout(
        &self,
        area: layout::Rect,
        work_size: u16,
        break_size: u16,
    ) -> (layout::Rect, layout::Rect, layout::Rect) {
        let (ascii_width, timer_width) = if !self.hide_image { (50, 50) } else { (0, 100) };
        let horizontal = layout::Layout::horizontal([
            layout::Constraint::Percentage(ascii_width),
            layout::Constraint::Percentage(timer_width),
        ]);
        let [left, right] = horizontal.areas(area);

        let left_layout = layout::Layout::vertical([
            layout::Constraint::Fill(1),
            layout::Constraint::Length(10),
            layout::Constraint::Fill(1),
        ]);
        let [_, lcenter, _] = left_layout.areas(left);

        let right_layout = layout::Layout::vertical([
            layout::Constraint::Fill(1),
            layout::Constraint::Length(work_size),
            layout::Constraint::Length(break_size),
            layout::Constraint::Fill(1),
        ]);
        let [_, rtop, rbottom, _] = right_layout.areas(right);

        (lcenter, rtop, rbottom)
    }

    fn get_block_widget(&self) -> widgets::Block<'_> {
        let start_pause = match self.pomo.is_running() {
            true => "Pause ",
            false => "Start ",
        };

        let title = text::Line::from(" Pomodoro ".bold());
        let instructions = text::Line::from(vec![
            start_pause.into(),
            "<S>".blue().bold(),
            " Reset ".into(),
            "<R>".blue().bold(),
            " Configure ".into(),
            "<C>".blue().bold(),
            " Quit ".into(),
            "<Q/Esc> ".blue().bold(),
        ]);
        widgets::Block::bordered()
            .title(title.centered())
            .title_bottom(instructions.centered())
            .border_set(symbols::border::THICK)
    }

    fn get_ascii_image_widget(&self) -> widgets::Paragraph<'_> {
        let ascii_image: Vec<text::Line> = match self.pomo.state() {
            pomodoro_tui::PomodoroState::Work => ascii_images::computer(),
            pomodoro_tui::PomodoroState::Break => ascii_images::sleeping_cat(),
        }
        .into_iter()
        .map(text::Line::from)
        .collect();

        widgets::Paragraph::new(ascii_image).alignment(layout::Alignment::Center)
    }

    fn get_timer_widgets(
        &self,
        work_pixel: tui_big_text::PixelSize,
        break_pixel: tui_big_text::PixelSize,
    ) -> (tui_big_text::BigText<'_>, tui_big_text::BigText<'_>) {
        let work_timer = tui_big_text::BigText::builder()
            .pixel_size(work_pixel)
            .lines(vec![self.pomo.work_time().blue().into()])
            .centered()
            .build();
        let break_timer = tui_big_text::BigText::builder()
            .pixel_size(break_pixel)
            .lines(vec![self.pomo.break_time().green().into()])
            .centered()
            .build();
        (work_timer, break_timer)
    }

    fn handle_key_event(&mut self, key_event: event::KeyEvent) {
        if self.menu_state != MenuState::None {
            // Handle menu navigation
            self.handle_menu_key_event(key_event);
        } else {
            // Handle normal app keys
            match key_event.code {
                event::KeyCode::Char('s') => {
                    self.pomo.start_or_pause();
                }
                event::KeyCode::Char('r') => {
                    self.pomo.reset();
                }
                event::KeyCode::Char('c') => {
                    self.menu_state = MenuState::MainMenu;
                    self.menu_selection = 0;
                }
                event::KeyCode::Esc => self.exit = true,
                event::KeyCode::Char('q') => self.exit = true,
                _ => (),
            }
        }
    }

    fn handle_menu_key_event(&mut self, key_event: event::KeyEvent) {
        match key_event.code {
            event::KeyCode::Up => {
                if self.menu_selection > 0 {
                    self.menu_selection -= 1;
                }
            }
            event::KeyCode::Down => {
                let max_items = match self.menu_state {
                    MenuState::MainMenu => 4, // 5 items (0-4)
                    MenuState::SelectWorkDuration | MenuState::SelectBreakDuration => {
                        DURATION_PRESETS.len() - 1
                    }
                    MenuState::ExtendWorkSession => DURATION_PRESETS.len(), // presets + "No, start break"
                    MenuState::SelectSound => {
                        let sound_count = self.get_sound_files().len();
                        if sound_count > 0 {
                            sound_count - 1
                        } else {
                            0
                        }
                    }
                    MenuState::None => 0,
                };
                if self.menu_selection < max_items {
                    self.menu_selection += 1;
                }
            }
            event::KeyCode::Enter => {
                self.handle_menu_selection();
            }
            event::KeyCode::Esc => {
                // Go back or close menu
                match self.menu_state {
                    MenuState::MainMenu => {
                        self.menu_state = MenuState::None;
                    }
                    MenuState::SelectWorkDuration | MenuState::SelectBreakDuration | MenuState::SelectSound => {
                        self.menu_state = MenuState::MainMenu;
                        self.menu_selection = 0;
                    }
                    MenuState::ExtendWorkSession => {
                        // Esc means "start break"
                        self.pomo.start_or_pause();
                        self.menu_state = MenuState::None;
                    }
                    MenuState::None => {}
                }
            }
            _ => (),
        }
    }

    fn handle_menu_selection(&mut self) {
        match self.menu_state {
            MenuState::MainMenu => {
                match self.menu_selection {
                    0 => {
                        // Change Work Duration
                        self.menu_state = MenuState::SelectWorkDuration;
                        self.menu_selection = 0;
                    }
                    1 => {
                        // Change Break Duration
                        self.menu_state = MenuState::SelectBreakDuration;
                        self.menu_selection = 0;
                    }
                    2 => {
                        // Toggle Auto-Start
                        self.pomo.toggle_auto_start();
                        // Stay in menu to show updated state
                    }
                    3 => {
                        // Change Notification Sound
                        self.menu_state = MenuState::SelectSound;
                        self.menu_selection = 0;
                    }
                    4 => {
                        // Back
                        self.menu_state = MenuState::None;
                    }
                    _ => {}
                }
            }
            MenuState::SelectWorkDuration => {
                if self.menu_selection < DURATION_PRESETS.len() {
                    let duration = DURATION_PRESETS[self.menu_selection];
                    self.pomo.set_work_duration(duration);
                    self.menu_state = MenuState::MainMenu;
                    self.menu_selection = 0;
                }
            }
            MenuState::SelectBreakDuration => {
                if self.menu_selection < DURATION_PRESETS.len() {
                    let duration = DURATION_PRESETS[self.menu_selection];
                    self.pomo.set_break_duration(duration);
                    self.menu_state = MenuState::MainMenu;
                    self.menu_selection = 0;
                }
            }
            MenuState::ExtendWorkSession => {
                if self.menu_selection < DURATION_PRESETS.len() {
                    // Extend work session
                    let duration = DURATION_PRESETS[self.menu_selection];
                    self.pomo.extend_work_session(duration);
                    self.menu_state = MenuState::None;
                } else {
                    // "No, start break" option selected
                    self.pomo.start_or_pause();
                    self.menu_state = MenuState::None;
                }
            }
            MenuState::SelectSound => {
                let sound_files = self.get_sound_files();
                if !sound_files.is_empty() && self.menu_selection < sound_files.len() {
                    let selected_sound = sound_files[self.menu_selection].clone();
                    self.pomo.set_sound(selected_sound);
                    self.menu_state = MenuState::MainMenu;
                    self.menu_selection = 0;
                }
            }
            MenuState::None => {}
        }
    }

    fn render_menu(&self, frame: &mut Frame, area: layout::Rect) {
        // Create centered popup area
        let popup_area = self.centered_rect(60, 60, area);

        // Clear the background
        frame.render_widget(widgets::Clear, popup_area);

        match self.menu_state {
            MenuState::MainMenu => {
                let auto_start_status = if self.pomo.auto_start() { "ON" } else { "OFF" };
                let auto_start_label = format!("Toggle Auto-Start ({})", auto_start_status);
                let items = vec![
                    "Change Work Duration",
                    "Change Break Duration",
                    &auto_start_label,
                    "Change Notification Sound",
                    "Back",
                ];
                self.render_menu_items(frame, popup_area, "Configuration Menu", &items);
            }
            MenuState::SelectWorkDuration | MenuState::SelectBreakDuration => {
                let title = match self.menu_state {
                    MenuState::SelectWorkDuration => "Select Work Duration (minutes)",
                    MenuState::SelectBreakDuration => "Select Break Duration (minutes)",
                    _ => "",
                };
                let items: Vec<String> = DURATION_PRESETS
                    .iter()
                    .map(|d| format!("{} minutes", d))
                    .collect();
                let items_refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
                self.render_menu_items(frame, popup_area, title, &items_refs);
            }
            MenuState::ExtendWorkSession => {
                let title = "Work Session Complete! Extend?";
                let items: Vec<String> = DURATION_PRESETS
                    .iter()
                    .map(|d| format!("Extend {} minutes", d))
                    .collect();
                let mut items_with_break = items.iter().map(|s| s.as_str()).collect::<Vec<&str>>();
                items_with_break.push("No, start break");
                self.render_menu_items(frame, popup_area, title, &items_with_break);
            }
            MenuState::SelectSound => {
                let title = "Select Notification Sound";
                let sound_files = self.get_sound_files();
                let items: Vec<String> = sound_files
                    .iter()
                    .filter_map(|p| {
                        p.file_name()
                            .and_then(|name| name.to_str())
                            .map(|s| s.to_string())
                    })
                    .collect();

                if items.is_empty() {
                    let empty_items = vec!["No sound files found in sounds/"];
                    self.render_menu_items(frame, popup_area, title, &empty_items);
                } else {
                    let items_refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
                    self.render_menu_items(frame, popup_area, title, &items_refs);
                }
            }
            MenuState::None => {}
        }
    }

    fn render_menu_items(&self, frame: &mut Frame, area: layout::Rect, title: &str, items: &[&str]) {
        let block = widgets::Block::bordered()
            .title(text::Line::from(title).centered())
            .border_set(symbols::border::THICK);

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let list_items: Vec<widgets::ListItem> = items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let content = if i == self.menu_selection {
                    text::Line::from(format!("> {}", item)).yellow().bold()
                } else {
                    text::Line::from(format!("  {}", item))
                };
                widgets::ListItem::new(content)
            })
            .collect();

        let list = widgets::List::new(list_items);
        frame.render_widget(list, inner);
    }

    fn centered_rect(&self, percent_x: u16, percent_y: u16, area: layout::Rect) -> layout::Rect {
        let popup_layout = layout::Layout::vertical([
            layout::Constraint::Percentage((100 - percent_y) / 2),
            layout::Constraint::Percentage(percent_y),
            layout::Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

        layout::Layout::horizontal([
            layout::Constraint::Percentage((100 - percent_x) / 2),
            layout::Constraint::Percentage(percent_x),
            layout::Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
    }

    fn get_sound_files(&self) -> Vec<PathBuf> {
        let sounds_dir = Path::new("sounds");
        let mut sound_files = Vec::new();

        if let Ok(entries) = fs::read_dir(sounds_dir) {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_file() {
                        let path = entry.path();
                        // Check if it's an audio file (basic check by extension)
                        if let Some(ext) = path.extension() {
                            let ext_str = ext.to_string_lossy().to_lowercase();
                            if ext_str == "mp3" || ext_str == "wav" || ext_str == "ogg" {
                                sound_files.push(path);
                            }
                        }
                    }
                }
            }
        }

        sound_files.sort();
        sound_files
    }
}
