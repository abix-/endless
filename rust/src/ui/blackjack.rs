//! Blackjack minigame — full-window popup opened from Casino building.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use rand::seq::SliceRandom;

use crate::resources::{Reputation, UiState};
use crate::world::WorldData;

// ============================================================================
// DATA TYPES
// ============================================================================

#[derive(Clone, Copy, Debug)]
pub struct Card {
    pub suit: u8, // 0-3: spades, hearts, diamonds, clubs
    pub rank: u8, // 1-13: Ace through King
}

impl Card {
    fn value(self) -> u8 {
        if self.rank >= 10 { 10 } else { self.rank }
    }

    fn rank_str(self) -> &'static str {
        match self.rank {
            1 => "A",
            2 => "2",
            3 => "3",
            4 => "4",
            5 => "5",
            6 => "6",
            7 => "7",
            8 => "8",
            9 => "9",
            10 => "10",
            11 => "J",
            12 => "Q",
            13 => "K",
            _ => "?",
        }
    }

    fn suit_char(self) -> &'static str {
        match self.suit {
            0 => "\u{2660}", // ♠
            1 => "\u{2665}", // ♥
            2 => "\u{2666}", // ♦
            _ => "\u{2663}", // ♣
        }
    }

    fn is_red(self) -> bool {
        self.suit == 1 || self.suit == 2
    }
}

#[derive(Clone, Default, Debug)]
pub struct Hand {
    pub cards: Vec<Card>,
    pub stood: bool,
    pub doubled: bool,
}

fn hand_value(cards: &[Card]) -> (u8, bool) {
    let hard: u8 = cards.iter().map(|c| c.value()).sum();
    let has_ace = cards.iter().any(|c| c.rank == 1);
    if has_ace && hard + 10 <= 21 {
        (hard + 10, true)
    } else {
        (hard, false)
    }
}

fn is_blackjack(cards: &[Card]) -> bool {
    cards.len() == 2 && hand_value(cards).0 == 21
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HandResult {
    Win,
    Lose,
    Push,
    Blackjack,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum BlackjackPhase {
    #[default]
    Betting,
    PlayerTurn,
    DealerTurn,
    Result,
}

const SHOE_DECKS: usize = 1;
const SHOE_SIZE: usize = SHOE_DECKS * 52;
const CUT_CARD: usize = 13;
const REPUTATION_PER_GOLD: f32 = 0.1;

// Card visual constants
const CARD_W: f32 = 44.0;
const CARD_H: f32 = 62.0;
const CARD_GAP: f32 = 6.0;
const CARD_ROUNDING: f32 = 4.0;

// Table colors
const FELT_GREEN: egui::Color32 = egui::Color32::from_rgba_premultiplied(25, 75, 38, 230);
const FELT_BORDER: egui::Color32 = egui::Color32::from_rgb(50, 110, 60);
const CARD_FACE: egui::Color32 = egui::Color32::from_rgb(250, 248, 240);
const CARD_BACK: egui::Color32 = egui::Color32::from_rgb(40, 60, 120);
const CARD_RED: egui::Color32 = egui::Color32::from_rgb(200, 30, 30);
const CARD_BLACK: egui::Color32 = egui::Color32::from_rgb(20, 20, 20);

// ============================================================================
// STATE
// ============================================================================

#[derive(Default)]
pub struct BlackjackState {
    pub phase: BlackjackPhase,
    pub shoe: Vec<Card>,
    pub player_hands: Vec<Hand>,
    pub active_hand: usize,
    pub dealer_hand: Hand,
    pub bet: i32,
    pub bet_input: String,
    pub opponent_faction: i32,
    pub opponent_town_idx: usize,
    pub results: Vec<HandResult>,
    pub message: String,
    pub total_bets: Vec<i32>,
}

impl BlackjackState {
    fn build_shoe(&mut self) {
        self.shoe.clear();
        for _ in 0..SHOE_DECKS {
            for suit in 0..4u8 {
                for rank in 1..=13u8 {
                    self.shoe.push(Card { suit, rank });
                }
            }
        }
        let mut rng = rand::rng();
        self.shoe.shuffle(&mut rng);
    }

    fn draw(&mut self) -> Card {
        if self.shoe.is_empty() {
            self.build_shoe();
        }
        self.shoe.pop().expect("shoe refilled")
    }

    fn needs_reshuffle(&self) -> bool {
        self.shoe.len() <= CUT_CARD
    }
}

// ============================================================================
// SYSTEM — standalone egui window
// ============================================================================

pub fn blackjack_window_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut town_access: crate::systemparams::TownAccess,
    mut reputation: ResMut<Reputation>,
    world_data: Res<WorldData>,
    mut state: Local<BlackjackState>,
) -> Result {
    if !ui_state.casino_open {
        return Ok(());
    }

    let ctx = contexts.ctx_mut()?;

    let frame = egui::Frame::new()
        .fill(FELT_GREEN)
        .stroke(egui::Stroke::new(3.0, FELT_BORDER))
        .inner_margin(egui::Margin::same(16))
        .corner_radius(egui::CornerRadius::same(8));

    let mut open = true;
    egui::Window::new("Casino — Blackjack")
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .default_width(560.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(frame)
        .show(ctx, |ui| {
            blackjack_content(
                ui,
                &mut state,
                &mut town_access,
                &mut reputation,
                &world_data,
            );
        });

    if !open {
        ui_state.casino_open = false;
    }

    Ok(())
}

// ============================================================================
// UI CONTENT
// ============================================================================

fn blackjack_content(
    ui: &mut egui::Ui,
    state: &mut BlackjackState,
    town_access: &mut crate::systemparams::TownAccess<'_, '_>,
    reputation: &mut Reputation,
    world_data: &WorldData,
) {
    let player_town = 0usize;

    let ai_towns: Vec<(usize, i32, &str)> = world_data
        .towns
        .iter()
        .enumerate()
        .filter(|(_, t)| t.faction >= 1)
        .map(|(idx, t)| (idx, t.faction, t.name.as_str()))
        .collect();

    if ai_towns.is_empty() {
        ui.colored_label(
            egui::Color32::from_rgb(200, 200, 150),
            "No AI factions to play against.",
        );
        return;
    }

    if state.opponent_faction == crate::constants::FACTION_NEUTRAL {
        if let Some(&(idx, faction, _)) = ai_towns.first() {
            state.opponent_faction = faction;
            state.opponent_town_idx = idx;
        }
    }

    let player_gold = town_access.gold(player_town as i32);
    let opponent_gold = town_access.gold(state.opponent_town_idx as i32);
    // How opponent feels about player (faction 0)
    let rep = reputation.get(state.opponent_faction, 0);

    // Header: opponent + gold + reputation
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Opponent:").color(egui::Color32::from_rgb(200, 200, 160)));
        egui::ComboBox::from_id_salt("bj_opponent")
            .selected_text(
                ai_towns
                    .iter()
                    .find(|t| t.1 == state.opponent_faction)
                    .map(|t| t.2)
                    .unwrap_or("???"),
            )
            .show_ui(ui, |ui| {
                for &(idx, faction, name) in &ai_towns {
                    if ui
                        .selectable_label(state.opponent_faction == faction, name)
                        .clicked()
                    {
                        state.opponent_faction = faction;
                        state.opponent_town_idx = idx;
                    }
                }
            });
    });

    ui.add_space(4.0);

    ui.horizontal(|ui| {
        let rep_color = if rep > 0.0 {
            egui::Color32::from_rgb(100, 200, 100)
        } else if rep < 0.0 {
            egui::Color32::from_rgb(200, 100, 100)
        } else {
            egui::Color32::from_rgb(180, 180, 180)
        };
        ui.label(
            egui::RichText::new(format!("Your gold: {player_gold}"))
                .color(egui::Color32::from_rgb(220, 200, 60))
                .strong(),
        );
        ui.add_space(20.0);
        ui.label(
            egui::RichText::new(format!("Their gold: {opponent_gold}"))
                .color(egui::Color32::from_rgb(180, 180, 180)),
        );
        ui.add_space(20.0);
        ui.label(egui::RichText::new("Rep:").color(egui::Color32::from_rgb(180, 180, 180)));
        ui.colored_label(rep_color, format!("{rep:+.1}"));
    });

    // Divider line
    ui.add_space(8.0);
    let rect = ui.available_rect_before_wrap();
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), rect.top()),
            egui::pos2(rect.right(), rect.top()),
        ],
        egui::Stroke::new(1.0, FELT_BORDER),
    );
    ui.add_space(10.0);

    match state.phase {
        BlackjackPhase::Betting => {
            betting_ui(ui, state, player_gold, opponent_gold);
        }
        BlackjackPhase::PlayerTurn => {
            render_table(ui, state, true);
            ui.add_space(12.0);
            player_turn_ui(ui, state, player_gold);
        }
        BlackjackPhase::DealerTurn => {
            play_dealer(state);
            resolve_hands(state, town_access, reputation, player_town);
            state.phase = BlackjackPhase::Result;
            render_table(ui, state, false);
            ui.add_space(12.0);
            result_ui(ui, state);
        }
        BlackjackPhase::Result => {
            render_table(ui, state, false);
            ui.add_space(12.0);
            result_ui(ui, state);
        }
    }

    // Shoe info footer
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(format!("Shoe: {}/{SHOE_SIZE}", state.shoe.len()))
            .color(egui::Color32::from_rgb(120, 140, 120))
            .size(11.0),
    );
}

// ============================================================================
// CARD RENDERING
// ============================================================================

fn draw_card(ui: &mut egui::Ui, card: Card) {
    let (_id, rect) = ui.allocate_space(egui::vec2(CARD_W, CARD_H));
    let painter = ui.painter();
    let rounding = egui::CornerRadius::same(CARD_ROUNDING as u8);

    painter.rect_filled(rect, rounding, CARD_FACE);
    painter.rect_stroke(
        rect,
        rounding,
        egui::Stroke::new(1.0, egui::Color32::from_rgb(160, 160, 160)),
        egui::StrokeKind::Outside,
    );

    let ink = if card.is_red() { CARD_RED } else { CARD_BLACK };

    painter.text(
        rect.left_top() + egui::vec2(4.0, 2.0),
        egui::Align2::LEFT_TOP,
        card.rank_str(),
        egui::FontId::proportional(14.0),
        ink,
    );

    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        card.suit_char(),
        egui::FontId::proportional(20.0),
        ink,
    );

    painter.text(
        rect.right_bottom() + egui::vec2(-4.0, -2.0),
        egui::Align2::RIGHT_BOTTOM,
        card.rank_str(),
        egui::FontId::proportional(14.0),
        ink,
    );
}

fn draw_card_back(ui: &mut egui::Ui) {
    let (_id, rect) = ui.allocate_space(egui::vec2(CARD_W, CARD_H));
    let painter = ui.painter();
    let rounding = egui::CornerRadius::same(CARD_ROUNDING as u8);

    painter.rect_filled(rect, rounding, CARD_BACK);
    painter.rect_stroke(
        rect,
        rounding,
        egui::Stroke::new(1.0, egui::Color32::from_rgb(80, 100, 160)),
        egui::StrokeKind::Outside,
    );

    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "\u{2666}",
        egui::FontId::proportional(18.0),
        egui::Color32::from_rgb(100, 120, 180),
    );
}

fn draw_hand(ui: &mut egui::Ui, cards: &[Card], hide_hole: bool) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = CARD_GAP;
        for (i, card) in cards.iter().enumerate() {
            if hide_hole && i == 1 {
                draw_card_back(ui);
            } else {
                draw_card(ui, *card);
            }
        }
    });
}

fn hand_value_label(ui: &mut egui::Ui, cards: &[Card], hide_hole: bool) {
    if hide_hole {
        let first = cards.first().map(|c| c.value()).unwrap_or(0);
        ui.label(
            egui::RichText::new(format!("showing {first}"))
                .color(egui::Color32::from_rgb(200, 200, 160)),
        );
    } else {
        let (val, soft) = hand_value(cards);
        let soft_str = if soft { " soft" } else { "" };
        let color = if val > 21 {
            egui::Color32::from_rgb(220, 80, 80)
        } else if val == 21 {
            egui::Color32::from_rgb(220, 200, 60)
        } else {
            egui::Color32::from_rgb(220, 220, 200)
        };
        let bust = if val > 21 { " BUST" } else { "" };
        ui.label(
            egui::RichText::new(format!("{val}{soft_str}{bust}"))
                .color(color)
                .strong(),
        );
    }
}

// ============================================================================
// TABLE RENDERING
// ============================================================================

fn render_table(ui: &mut egui::Ui, state: &BlackjackState, hide_hole: bool) {
    // Dealer
    ui.label(
        egui::RichText::new("DEALER")
            .color(egui::Color32::from_rgb(180, 180, 140))
            .size(12.0),
    );
    draw_hand(ui, &state.dealer_hand.cards, hide_hole);
    hand_value_label(ui, &state.dealer_hand.cards, hide_hole);

    ui.add_space(16.0);

    // Player hand(s)
    for (h_idx, hand) in state.player_hands.iter().enumerate() {
        let is_active = state.phase == BlackjackPhase::PlayerTurn && h_idx == state.active_hand;
        let label = if state.player_hands.len() > 1 {
            let marker = if is_active { "> " } else { "  " };
            format!("{marker}HAND {}", h_idx + 1)
        } else {
            "YOUR HAND".to_string()
        };
        let label_color = if is_active {
            egui::Color32::from_rgb(220, 220, 100)
        } else {
            egui::Color32::from_rgb(180, 180, 140)
        };
        ui.label(egui::RichText::new(&label).color(label_color).size(12.0));
        draw_hand(ui, &hand.cards, false);
        ui.horizontal(|ui| {
            hand_value_label(ui, &hand.cards, false);
            if hand.doubled {
                ui.label(
                    egui::RichText::new("DOUBLED")
                        .color(egui::Color32::from_rgb(200, 160, 60))
                        .size(11.0),
                );
            }
            if state.player_hands.len() > 1 {
                if let Some(&hbet) = state.total_bets.get(h_idx) {
                    ui.label(
                        egui::RichText::new(format!("bet: {hbet}g"))
                            .color(egui::Color32::from_rgb(160, 160, 140))
                            .size(11.0),
                    );
                }
            }
        });
    }
}

// ============================================================================
// PHASE UIs
// ============================================================================

fn betting_ui(ui: &mut egui::Ui, state: &mut BlackjackState, player_gold: i32, opponent_gold: i32) {
    ui.add_space(8.0);
    let rules_color = egui::Color32::from_rgb(160, 170, 150);
    egui::CollapsingHeader::new(egui::RichText::new("Rules").color(rules_color).size(12.0))
        .default_open(false)
        .show(ui, |ui| {
            let txt = "Single deck, reshuffled at 75%. Beat the dealer without going over 21.\n\
                       Cards 2-10 = face value, J/Q/K = 10, Ace = 1 or 11.\n\
                       Blackjack (Ace + 10-card) pays 3:2. Dealer stands on 17.\n\
                       Double: double bet, receive exactly one more card.\n\
                       Split: matching cards become two hands at original bet each.\n\
                       Push (tie) returns your bet. Winning costs the opponent gold and reputation.";
            ui.label(egui::RichText::new(txt).color(rules_color).size(11.0));
        });
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new("Place your bet")
            .color(egui::Color32::from_rgb(220, 220, 180))
            .size(16.0),
    );
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Bet:").color(egui::Color32::from_rgb(200, 200, 160)));
        ui.add(
            egui::TextEdit::singleline(&mut state.bet_input)
                .desired_width(80.0)
                .font(egui::TextStyle::Monospace),
        );
        ui.label(egui::RichText::new("gold").color(egui::Color32::from_rgb(160, 160, 140)));
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for amount in [1, 5, 10, 25, 50, 100] {
            let btn = egui::Button::new(
                egui::RichText::new(format!("{amount}"))
                    .color(egui::Color32::from_rgb(220, 200, 60))
                    .size(13.0),
            )
            .fill(egui::Color32::from_rgb(40, 60, 35));
            if ui.add(btn).clicked() {
                state.bet_input = amount.to_string();
            }
        }
        let max = player_gold.min(opponent_gold);
        let btn = egui::Button::new(
            egui::RichText::new("All-in")
                .color(egui::Color32::from_rgb(220, 100, 100))
                .size(13.0),
        )
        .fill(egui::Color32::from_rgb(60, 35, 35));
        if ui.add(btn).clicked() {
            state.bet_input = max.to_string();
        }
    });

    let bet: i32 = state.bet_input.trim().parse().unwrap_or(0);
    let can_deal = bet >= 1 && bet <= player_gold && bet <= opponent_gold;

    if !can_deal && bet > 0 {
        ui.add_space(4.0);
        if bet > player_gold {
            ui.colored_label(egui::Color32::from_rgb(200, 100, 100), "Not enough gold!");
        } else if bet > opponent_gold {
            ui.colored_label(
                egui::Color32::from_rgb(200, 100, 100),
                "Opponent doesn't have enough gold!",
            );
        }
    }

    ui.add_space(12.0);
    let deal_btn = egui::Button::new(
        egui::RichText::new("  DEAL  ")
            .color(egui::Color32::WHITE)
            .size(18.0)
            .strong(),
    )
    .fill(egui::Color32::from_rgb(40, 100, 50));
    if ui.add_enabled(can_deal, deal_btn).clicked() {
        state.bet = bet;
        deal(state);
    }
}

fn player_turn_ui(ui: &mut egui::Ui, state: &mut BlackjackState, player_gold: i32) {
    let ah = state.active_hand;
    let (val, _) = hand_value(&state.player_hands[ah].cards);

    if val > 21 {
        advance_hand(state);
        return;
    }

    let num_cards = state.player_hands[ah].cards.len();
    let is_doubled = state.player_hands[ah].doubled;
    let hand_bet = state.total_bets[ah];
    let can_double = num_cards == 2 && !is_doubled && player_gold >= hand_bet;
    let can_split = num_cards == 2
        && state.player_hands[ah].cards[0].value() == state.player_hands[ah].cards[1].value()
        && player_gold >= state.bet;

    let btn_size = 15.0;
    let btn_fill = egui::Color32::from_rgb(45, 70, 45);

    let mut action = None;
    ui.horizontal(|ui| {
        if ui
            .add(
                egui::Button::new(egui::RichText::new("  Hit  ").size(btn_size).strong())
                    .fill(btn_fill),
            )
            .clicked()
        {
            action = Some(0);
        }
        if ui
            .add(
                egui::Button::new(egui::RichText::new(" Stand ").size(btn_size).strong())
                    .fill(btn_fill),
            )
            .clicked()
        {
            action = Some(1);
        }
        let dbl_btn =
            egui::Button::new(egui::RichText::new("Double").size(btn_size).strong()).fill(btn_fill);
        if ui.add_enabled(can_double, dbl_btn).clicked() {
            action = Some(2);
        }
        let split_btn = egui::Button::new(egui::RichText::new(" Split ").size(btn_size).strong())
            .fill(btn_fill);
        if ui.add_enabled(can_split, split_btn).clicked() {
            action = Some(3);
        }
    });

    match action {
        Some(0) => {
            let card = state.draw();
            state.player_hands[ah].cards.push(card);
            let (new_val, _) = hand_value(&state.player_hands[ah].cards);
            if new_val > 21 {
                advance_hand(state);
            }
        }
        Some(1) => {
            state.player_hands[ah].stood = true;
            advance_hand(state);
        }
        Some(2) => {
            state.total_bets[ah] *= 2;
            state.player_hands[ah].doubled = true;
            let card = state.draw();
            state.player_hands[ah].cards.push(card);
            state.player_hands[ah].stood = true;
            advance_hand(state);
        }
        Some(3) => {
            let second_card = state.player_hands[ah]
                .cards
                .pop()
                .expect("split card exists");
            let new_card_a = state.draw();
            state.player_hands[ah].cards.push(new_card_a);
            let new_card_b = state.draw();
            let new_hand = Hand {
                cards: vec![second_card, new_card_b],
                stood: false,
                doubled: false,
            };
            state.player_hands.push(new_hand);
            state.total_bets.push(state.bet);
        }
        _ => {}
    }
}

fn result_ui(ui: &mut egui::Ui, state: &mut BlackjackState) {
    let has_win = state
        .results
        .iter()
        .any(|r| matches!(r, HandResult::Win | HandResult::Blackjack));
    let has_lose = state.results.iter().any(|r| matches!(r, HandResult::Lose));
    let color = if has_win && !has_lose {
        egui::Color32::from_rgb(80, 220, 80)
    } else if has_lose && !has_win {
        egui::Color32::from_rgb(220, 80, 80)
    } else {
        egui::Color32::from_rgb(220, 220, 80)
    };

    ui.colored_label(
        color,
        egui::RichText::new(&state.message).size(15.0).strong(),
    );

    ui.add_space(8.0);
    let btn = egui::Button::new(
        egui::RichText::new("Play Again")
            .color(egui::Color32::WHITE)
            .size(15.0)
            .strong(),
    )
    .fill(egui::Color32::from_rgb(40, 100, 50));
    if ui.add(btn).clicked() {
        state.phase = BlackjackPhase::Betting;
    }
}

// ============================================================================
// GAME LOGIC (unchanged)
// ============================================================================

fn deal(state: &mut BlackjackState) {
    if state.needs_reshuffle() || state.shoe.is_empty() {
        state.build_shoe();
    }

    state.dealer_hand = Hand::default();
    state.player_hands = vec![Hand::default()];
    state.active_hand = 0;
    state.results.clear();
    state.message.clear();
    state.total_bets = vec![state.bet];

    let c1 = state.draw();
    state.player_hands[0].cards.push(c1);
    let c2 = state.draw();
    state.dealer_hand.cards.push(c2);
    let c3 = state.draw();
    state.player_hands[0].cards.push(c3);
    let c4 = state.draw();
    state.dealer_hand.cards.push(c4);

    let player_bj = is_blackjack(&state.player_hands[0].cards);
    let dealer_bj = is_blackjack(&state.dealer_hand.cards);

    if player_bj || dealer_bj {
        state.phase = BlackjackPhase::DealerTurn;
    } else {
        state.phase = BlackjackPhase::PlayerTurn;
    }
}

fn advance_hand(state: &mut BlackjackState) {
    let next = state.active_hand + 1;
    if next < state.player_hands.len() {
        state.active_hand = next;
        let (val, _) = hand_value(&state.player_hands[next].cards);
        if val > 21 || state.player_hands[next].stood {
            advance_hand(state);
        }
    } else {
        state.phase = BlackjackPhase::DealerTurn;
    }
}

fn play_dealer(state: &mut BlackjackState) {
    let all_busted = state
        .player_hands
        .iter()
        .all(|h| hand_value(&h.cards).0 > 21);
    if all_busted {
        return;
    }

    loop {
        let (val, _) = hand_value(&state.dealer_hand.cards);
        if val >= 17 {
            break;
        }
        let card = state.draw();
        state.dealer_hand.cards.push(card);
    }
}

fn resolve_hands(
    state: &mut BlackjackState,
    town_access: &mut crate::systemparams::TownAccess<'_, '_>,
    reputation: &mut Reputation,
    player_town: usize,
) {
    let (dealer_val, _) = hand_value(&state.dealer_hand.cards);
    let dealer_bj = is_blackjack(&state.dealer_hand.cards);
    let dealer_bust = dealer_val > 21;

    state.results.clear();
    let mut total_net: i32 = 0;

    for (h_idx, hand) in state.player_hands.iter().enumerate() {
        let (player_val, _) = hand_value(&hand.cards);
        let player_bj = is_blackjack(&hand.cards);
        let player_bust = player_val > 21;
        let hand_bet = state.total_bets.get(h_idx).copied().unwrap_or(state.bet);

        let result = if player_bust {
            HandResult::Lose
        } else if player_bj && dealer_bj {
            HandResult::Push
        } else if player_bj {
            HandResult::Blackjack
        } else if dealer_bj {
            HandResult::Lose
        } else if dealer_bust || player_val > dealer_val {
            HandResult::Win
        } else if player_val < dealer_val {
            HandResult::Lose
        } else {
            HandResult::Push
        };

        let net = match result {
            HandResult::Win => hand_bet,
            HandResult::Blackjack => (hand_bet * 3) / 2,
            HandResult::Lose => -hand_bet,
            HandResult::Push => 0,
        };
        total_net += net;
        state.results.push(result);
    }

    if let Some(mut pg) = town_access.gold_mut(player_town as i32) {
        pg.0 += total_net;
    }
    if let Some(mut og) = town_access.gold_mut(state.opponent_town_idx as i32) {
        og.0 -= total_net;
    }

    // Player is faction 0 — update how opponent feels about player
    if let Some(row) = reputation.values.get_mut(state.opponent_faction as usize) {
        if let Some(rep) = row.get_mut(0) {
            let cap = crate::constants::SOFT_CAP as f32;
            *rep = (*rep - total_net as f32 * REPUTATION_PER_GOLD).clamp(-cap, cap);
        }
    }

    let mut parts = Vec::new();
    for (i, result) in state.results.iter().enumerate() {
        let hand_bet = state.total_bets.get(i).copied().unwrap_or(state.bet);
        let prefix = if state.results.len() > 1 {
            format!("Hand {}: ", i + 1)
        } else {
            String::new()
        };
        match result {
            HandResult::Win => parts.push(format!("{prefix}Win! +{hand_bet}g")),
            HandResult::Blackjack => {
                let payout = (hand_bet * 3) / 2;
                parts.push(format!("{prefix}BLACKJACK! +{payout}g"));
            }
            HandResult::Lose => parts.push(format!("{prefix}Lose. -{hand_bet}g")),
            HandResult::Push => parts.push(format!("{prefix}Push.")),
        }
    }
    let rep_delta = -(total_net as f32 * REPUTATION_PER_GOLD);
    if rep_delta.abs() > 0.01 {
        parts.push(format!("Rep: {rep_delta:+.1}"));
    }
    state.message = parts.join("  |  ");
}
