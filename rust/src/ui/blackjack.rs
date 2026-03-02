//! Blackjack minigame — play against AI factions for real gold + reputation.

use bevy_egui::egui;
use rand::seq::SliceRandom;

use crate::resources::{GoldStorage, Reputation};
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

    fn display(self) -> String {
        let rank = match self.rank {
            1 => "A".to_string(),
            11 => "J".to_string(),
            12 => "Q".to_string(),
            13 => "K".to_string(),
            n => n.to_string(),
        };
        let suit = match self.suit {
            0 => "\u{2660}", // ♠
            1 => "\u{2665}", // ♥
            2 => "\u{2666}", // ♦
            _ => "\u{2663}", // ♣
        };
        format!("{rank}{suit}")
    }

    fn suit_color(self) -> egui::Color32 {
        match self.suit {
            1 | 2 => egui::Color32::from_rgb(220, 50, 50), // hearts/diamonds = red
            _ => egui::Color32::from_rgb(220, 220, 220),    // spades/clubs = white-ish
        }
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

const SHOE_DECKS: usize = 3;
const SHOE_SIZE: usize = SHOE_DECKS * 52;
const CUT_CARD: usize = 39; // reshuffle when <= 39 cards remain (~75% dealt)
const REPUTATION_PER_GOLD: f32 = 0.1;

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
    pub total_bets: Vec<i32>, // per-hand bet (doubles may differ)
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
        self.shoe.pop().unwrap()
    }

    fn needs_reshuffle(&self) -> bool {
        self.shoe.len() <= CUT_CARD
    }
}

// ============================================================================
// UI
// ============================================================================

pub fn blackjack_content(
    ui: &mut egui::Ui,
    state: &mut BlackjackState,
    gold_storage: &mut GoldStorage,
    reputation: &mut Reputation,
    world_data: &WorldData,
) {
    let player_town = 0usize; // player is always town 0

    // Collect AI factions (faction >= 1)
    let ai_towns: Vec<(usize, i32, &str)> = world_data
        .towns
        .iter()
        .enumerate()
        .filter(|(_, t)| t.faction >= 1)
        .map(|(idx, t)| (idx, t.faction, t.name.as_str()))
        .collect();

    if ai_towns.is_empty() {
        ui.label("No AI factions to play against.");
        return;
    }

    // Default opponent selection
    if state.opponent_faction == 0 {
        if let Some(&(idx, faction, _)) = ai_towns.first() {
            state.opponent_faction = faction;
            state.opponent_town_idx = idx;
        }
    }

    let player_gold = gold_storage.gold.get(player_town).copied().unwrap_or(0);
    let opponent_gold = gold_storage
        .gold
        .get(state.opponent_town_idx)
        .copied()
        .unwrap_or(0);
    let rep = reputation
        .values
        .get(state.opponent_faction as usize)
        .copied()
        .unwrap_or(0.0);

    // Header
    ui.horizontal(|ui| {
        ui.label("Opponent:");
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

    let rep_color = if rep > 0.0 {
        egui::Color32::from_rgb(100, 200, 100)
    } else if rep < 0.0 {
        egui::Color32::from_rgb(200, 100, 100)
    } else {
        egui::Color32::GRAY
    };
    ui.horizontal(|ui| {
        ui.label("Reputation:");
        ui.colored_label(rep_color, format!("{rep:+.1}"));
        ui.separator();
        ui.label(format!("Your gold: {player_gold}"));
        ui.separator();
        ui.label(format!("Their gold: {opponent_gold}"));
    });

    ui.separator();

    match state.phase {
        BlackjackPhase::Betting => {
            betting_ui(ui, state, player_gold, opponent_gold);
        }
        BlackjackPhase::PlayerTurn => {
            render_table(ui, state, true);
            player_turn_ui(ui, state, player_gold);
        }
        BlackjackPhase::DealerTurn => {
            // Auto-play dealer
            play_dealer(state);
            // Resolve all hands
            resolve_hands(state, gold_storage, reputation, player_town);
            state.phase = BlackjackPhase::Result;
            render_table(ui, state, false);
            result_ui(ui, state);
        }
        BlackjackPhase::Result => {
            render_table(ui, state, false);
            result_ui(ui, state);
        }
    }

    // Shoe info
    ui.separator();
    ui.small(format!(
        "Shoe: {}/{SHOE_SIZE} cards remaining",
        state.shoe.len()
    ));
}

fn betting_ui(ui: &mut egui::Ui, state: &mut BlackjackState, player_gold: i32, opponent_gold: i32) {
    ui.horizontal(|ui| {
        ui.label("Bet:");
        ui.add(egui::TextEdit::singleline(&mut state.bet_input).desired_width(60.0));
    });

    // Quick bet buttons
    ui.horizontal(|ui| {
        for amount in [1, 5, 10, 25, 50, 100] {
            if ui.small_button(format!("{amount}")).clicked() {
                state.bet_input = amount.to_string();
            }
        }
        if ui.small_button("All-in").clicked() {
            let max = player_gold.min(opponent_gold);
            state.bet_input = max.to_string();
        }
    });

    let bet: i32 = state.bet_input.trim().parse().unwrap_or(0);
    let can_deal = bet >= 1 && bet <= player_gold && bet <= opponent_gold;

    if !can_deal && bet > 0 {
        if bet > player_gold {
            ui.colored_label(egui::Color32::from_rgb(200, 100, 100), "Not enough gold!");
        } else if bet > opponent_gold {
            ui.colored_label(
                egui::Color32::from_rgb(200, 100, 100),
                "Opponent doesn't have enough gold!",
            );
        }
    }

    if ui
        .add_enabled(can_deal, egui::Button::new("Deal"))
        .clicked()
    {
        state.bet = bet;
        deal(state);
    }
}

fn deal(state: &mut BlackjackState) {
    // Reshuffle if needed
    if state.needs_reshuffle() || state.shoe.is_empty() {
        state.build_shoe();
    }

    state.dealer_hand = Hand::default();
    state.player_hands = vec![Hand::default()];
    state.active_hand = 0;
    state.results.clear();
    state.message.clear();
    state.total_bets = vec![state.bet];

    // Deal: player, dealer, player, dealer
    let c1 = state.draw();
    state.player_hands[0].cards.push(c1);
    let c2 = state.draw();
    state.dealer_hand.cards.push(c2);
    let c3 = state.draw();
    state.player_hands[0].cards.push(c3);
    let c4 = state.draw();
    state.dealer_hand.cards.push(c4);

    // Check naturals
    let player_bj = is_blackjack(&state.player_hands[0].cards);
    let dealer_bj = is_blackjack(&state.dealer_hand.cards);

    if player_bj || dealer_bj {
        // Skip to dealer turn (resolves instantly)
        state.phase = BlackjackPhase::DealerTurn;
    } else {
        state.phase = BlackjackPhase::PlayerTurn;
    }
}

fn render_table(ui: &mut egui::Ui, state: &BlackjackState, hide_hole: bool) {
    // Dealer hand
    ui.horizontal(|ui| {
        ui.label("Dealer: ");
        for (i, card) in state.dealer_hand.cards.iter().enumerate() {
            if hide_hole && i == 1 {
                ui.colored_label(egui::Color32::GRAY, "[??]");
            } else {
                ui.colored_label(card.suit_color(), format!("[{}]", card.display()));
            }
        }
        if !hide_hole {
            let (val, _) = hand_value(&state.dealer_hand.cards);
            ui.label(format!("= {val}"));
        } else {
            // Show only first card value
            let first = state.dealer_hand.cards.first().map(|c| c.value()).unwrap_or(0);
            ui.label(format!("showing {first}"));
        }
    });

    ui.add_space(4.0);

    // Player hands
    for (h_idx, hand) in state.player_hands.iter().enumerate() {
        let is_active = state.phase == BlackjackPhase::PlayerTurn && h_idx == state.active_hand;
        let prefix = if state.player_hands.len() > 1 {
            let marker = if is_active { ">" } else { " " };
            format!("{marker}Hand {}: ", h_idx + 1)
        } else {
            "You: ".to_string()
        };

        ui.horizontal(|ui| {
            ui.label(&prefix);
            for card in &hand.cards {
                ui.colored_label(card.suit_color(), format!("[{}]", card.display()));
            }
            let (val, soft) = hand_value(&hand.cards);
            let soft_str = if soft { " (soft)" } else { "" };
            if val > 21 {
                ui.colored_label(egui::Color32::from_rgb(200, 100, 100), format!("= {val} BUST"));
            } else {
                ui.label(format!("= {val}{soft_str}"));
            }
            if hand.doubled {
                ui.small("(doubled)");
            }
            // Show per-hand bet if multiple hands
            if state.player_hands.len() > 1 {
                if let Some(&hbet) = state.total_bets.get(h_idx) {
                    ui.small(format!("[bet: {hbet}]"));
                }
            }
        });
    }
}

fn player_turn_ui(ui: &mut egui::Ui, state: &mut BlackjackState, player_gold: i32) {
    let ah = state.active_hand;
    let (val, _) = hand_value(&state.player_hands[ah].cards);

    if val > 21 {
        advance_hand(state);
        return;
    }

    // Compute button availability before entering the closure
    let num_cards = state.player_hands[ah].cards.len();
    let is_doubled = state.player_hands[ah].doubled;
    let hand_bet = state.total_bets[ah];
    let can_double = num_cards == 2 && !is_doubled && player_gold >= hand_bet;
    let can_split = num_cards == 2
        && state.player_hands[ah].cards[0].value() == state.player_hands[ah].cards[1].value()
        && player_gold >= state.bet;

    let mut action = None;
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button("Hit").clicked() {
            action = Some(0);
        }
        if ui.button("Stand").clicked() {
            action = Some(1);
        }
        if ui.add_enabled(can_double, egui::Button::new("Double")).clicked() {
            action = Some(2);
        }
        if ui.add_enabled(can_split, egui::Button::new("Split")).clicked() {
            action = Some(3);
        }
    });

    // Process action outside the closure to avoid borrow conflicts
    match action {
        Some(0) => {
            // Hit
            let card = state.draw();
            state.player_hands[ah].cards.push(card);
            let (new_val, _) = hand_value(&state.player_hands[ah].cards);
            if new_val > 21 {
                advance_hand(state);
            }
        }
        Some(1) => {
            // Stand
            state.player_hands[ah].stood = true;
            advance_hand(state);
        }
        Some(2) => {
            // Double
            state.total_bets[ah] *= 2;
            state.player_hands[ah].doubled = true;
            let card = state.draw();
            state.player_hands[ah].cards.push(card);
            state.player_hands[ah].stood = true;
            advance_hand(state);
        }
        Some(3) => {
            // Split
            let second_card = state.player_hands[ah].cards.pop().unwrap();
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

fn advance_hand(state: &mut BlackjackState) {
    // Move to next unfinished hand, or dealer turn
    let next = state.active_hand + 1;
    if next < state.player_hands.len() {
        state.active_hand = next;
        // Check if this hand is already busted/stood
        let (val, _) = hand_value(&state.player_hands[next].cards);
        if val > 21 || state.player_hands[next].stood {
            advance_hand(state);
        }
    } else {
        // All hands done
        state.phase = BlackjackPhase::DealerTurn;
    }
}

fn play_dealer(state: &mut BlackjackState) {
    // Check if all player hands busted (dealer doesn't need to draw)
    let all_busted = state
        .player_hands
        .iter()
        .all(|h| hand_value(&h.cards).0 > 21);
    if all_busted {
        return;
    }

    // Dealer hits until 17+
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
    gold: &mut GoldStorage,
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
        } else if dealer_bust {
            HandResult::Win
        } else if player_val > dealer_val {
            HandResult::Win
        } else if player_val < dealer_val {
            HandResult::Lose
        } else {
            HandResult::Push
        };

        let net = match result {
            HandResult::Win => hand_bet,
            HandResult::Blackjack => (hand_bet * 3) / 2, // 3:2 payout
            HandResult::Lose => -hand_bet,
            HandResult::Push => 0,
        };
        total_net += net;
        state.results.push(result);
    }

    // Transfer gold
    if let Some(pg) = gold.gold.get_mut(player_town) {
        *pg += total_net;
    }
    if let Some(og) = gold.gold.get_mut(state.opponent_town_idx) {
        *og -= total_net;
    }

    // Reputation: winning from them = they like you less
    if let Some(rep) = reputation.values.get_mut(state.opponent_faction as usize) {
        *rep = (*rep - total_net as f32 * REPUTATION_PER_GOLD).clamp(-100.0, 100.0);
    }

    // Build result message
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
    state.message = parts.join("  ");
}

fn result_ui(ui: &mut egui::Ui, state: &mut BlackjackState) {
    ui.add_space(4.0);

    // Color the message based on net outcome
    let has_win = state.results.iter().any(|r| matches!(r, HandResult::Win | HandResult::Blackjack));
    let has_lose = state.results.iter().any(|r| matches!(r, HandResult::Lose));
    let color = if has_win && !has_lose {
        egui::Color32::from_rgb(100, 200, 100)
    } else if has_lose && !has_win {
        egui::Color32::from_rgb(200, 100, 100)
    } else {
        egui::Color32::from_rgb(200, 200, 100)
    };

    ui.colored_label(color, &state.message);

    if ui.button("Play Again").clicked() {
        state.phase = BlackjackPhase::Betting;
    }
}
