//! Report query tests. Fixture hands are hand-authored with known money and
//! flags; every expected statistic is computed by hand in the test body and
//! asserted against [`texture::storage::queries`].

use texture::domain::{
    Card, FilterSpec, Game, GroupBy, HeroHandAnalysis, ParsedHand, ParsedSeat, Position, Site,
    Street,
};
use texture::money::Currency;
use texture::money::Money;
use texture::storage::queries as q;
use texture::storage::{self, Db, hands};

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

const HERO: &str = "Hero";

/// One hero hand to insert.
struct Fixture {
    hand_id: String,
    /// UTC unix seconds.
    ts: i64,
    profile_id: i64,
    position: Position,
    table_size: u8,
    sb: i64,
    bb: i64,
    /// Hero net in minor units. adjusted == net, luck 0 (EV-neutral fixtures).
    net: i64,
    went_to_showdown: bool,
    saw_flop: bool,
    vpip: bool,
    aggressive: [u32; 4],
}

impl Fixture {
    fn new(id: &str, ts: i64) -> Self {
        Fixture {
            hand_id: id.to_string(),
            ts,
            profile_id: 1,
            position: Position::Co,
            table_size: 6,
            sb: 25,
            bb: 50,
            net: 0,
            went_to_showdown: false,
            saw_flop: false,
            vpip: false,
            aggressive: [0; 4],
        }
    }
}

fn money(minor: i64) -> Money {
    Money::new(minor, Currency::Eur)
}

fn ts_of(y: i32, m: u32, d: u32, h: u32) -> i64 {
    use chrono::TimeZone;
    chrono::Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap().timestamp()
}

/// Build a `ParsedHand`. The hero is seat 2 named `"Hero"`; two villain seats
/// split the negation of the hero's result so the per-hand sum stays zero.
fn build_parsed(f: &Fixture) -> ParsedHand {
    let villain_a = -f.net / 2;
    let seats = vec![
        ParsedSeat {
            seat: 1,
            screen_name: "VillainA".into(),
            starting_stack: money(20_000),
            position: Position::Btn,
            contributed: 100,
            result: money(villain_a),
            shown_cards: vec![],
            sat_out: false,
        },
        ParsedSeat {
            seat: 2,
            screen_name: HERO.into(),
            starting_stack: money(20_000),
            position: f.position,
            contributed: 100,
            result: money(f.net),
            shown_cards: vec![],
            sat_out: false,
        },
        ParsedSeat {
            seat: 3,
            screen_name: "VillainB".into(),
            starting_stack: money(20_000),
            position: Position::Sb,
            contributed: 100,
            result: money(f.net - villain_a),
            shown_cards: vec![],
            sat_out: false,
        },
    ];
    ParsedHand {
        site: Site::PokerStars,
        hand_id: f.hand_id.clone(),
        game: Game::NoLimitHoldem,
        currency: Currency::Eur,
        small_blind: money(f.sb),
        big_blind: money(f.bb),
        table_name: "Test Table".into(),
        table_size: f.table_size,
        button_seat: 1,
        timestamp_utc: chrono::DateTime::from_timestamp(f.ts, 0).unwrap(),
        local_timestamp: "2024/01/03 12:00:00 UTC".into(),
        tz_token: "UTC".into(),
        board: vec![
            Card { rank: 2, suit: 'c' },
            Card { rank: 7, suit: 'd' },
            Card { rank: 9, suit: 'h' },
        ],
        streets_dealt: if f.saw_flop {
            vec![Street::Preflop, Street::Flop, Street::Turn]
        } else {
            vec![Street::Preflop]
        },
        seats,
        actions: vec![],
        pot_total: money(300),
        rake: money(3),
        jackpot_drop: money(0),
        went_to_showdown: f.went_to_showdown,
    }
}

/// Analysis row mirroring the fixture, as the stats module would derive it.
fn build_analysis(f: &Fixture) -> HeroHandAnalysis {
    HeroHandAnalysis {
        hand_id: f.hand_id.clone(),
        profile_id: f.profile_id,
        player_id: 0, // resolved at insert; not used by queries
        position: f.position,
        table_size: f.table_size,
        currency: Currency::Eur,
        small_blind: f.sb,
        big_blind: f.bb,
        timestamp_utc: chrono::DateTime::from_timestamp(f.ts, 0).unwrap(),
        net_result: f.net,
        adjusted_result: f.net,
        luck_delta: 0,
        rake_attributed: 3,
        jackpot_attributed: 0,
        contributed: 100,
        won_amount: f.net.max(0),
        sd_winnings: if f.went_to_showdown { f.net.max(0) } else { 0 },
        non_sd_winnings: if f.went_to_showdown { 0 } else { f.net.max(0) },
        went_to_showdown: f.went_to_showdown,
        won_at_showdown: f.net > 0 && f.went_to_showdown,
        saw_flop: f.saw_flop,
        saw_turn: f.saw_flop,
        saw_river: false,
        wwsf_opportunity: f.saw_flop,
        won_when_saw_flop: f.saw_flop && f.net > 0,
        vpip: f.vpip,
        pfr: false,
        limp: false,
        rfi: false,
        rfi_opportunity: false,
        three_bet: false,
        three_bet_opportunity: false,
        faced_three_bet: false,
        folded_to_three_bet: false,
        four_bet: false,
        four_bet_opportunity: false,
        faced_four_bet: false,
        folded_to_four_bet: false,
        cold_call: false,
        cold_call_opportunity: false,
        steal_attempt: false,
        steal_opportunity: false,
        faced_steal: false,
        folded_to_steal: false,
        cbet_opportunity: [false; 3],
        cbet: [false; 3],
        faced_cbet: [false; 3],
        folded_to_cbet: [false; 3],
        check_raise_opportunity: [false; 3],
        check_raise: [false; 3],
        probe_opportunity: [false; 3],
        probe: [false; 3],
        delayed_cbet_opportunity: [false; 3],
        delayed_cbet: [false; 3],
        aggressive_actions: f.aggressive,
        passive_actions: [0; 4],
    }
}

fn insert(db: &Db, f: &Fixture) {
    let parsed = build_parsed(f);
    let mut analysis = build_analysis(f);
    // hero_hands.player_id has a FK to players.id; resolve the real id.
    analysis.player_id = db.upsert_player(HERO).unwrap();
    let raw = format!("PokerStars Hand #{}: raw fixture", f.hand_id);
    hands::upsert_hand(db, &parsed, Some(&analysis), &raw, "test.txt").unwrap();
}

// ---------------------------------------------------------------------------
// The standard population used by the filter-dimension tests
// ---------------------------------------------------------------------------

/// Standard population (all UTC 2024):
///
/// | hand  | day          | profile | stake        | size | position | net  |
/// |-------|--------------|---------|--------------|------|----------|------|
/// | A     | 2024-01-03   | 1       | €0.25/€0.50  | 6    | CO       | +500 |
/// | B     | 2024-01-03   | 1       | €0.25/€0.50  | 6    | BTN      | -250 |
/// | C     | 2024-01-04   | 1       | €0.25/€0.50  | 6    | CO       | +100 |
/// | D     | 2024-01-04   | 2       | €0.25/€0.50  | 6    | BTN      | +75  |
/// | E     | 2024-01-04   | 1       | €0.10/€0.25  | 9    | UTG      | -25  |
/// | F     | 2024-01-05   | 1       | €0.25/€0.50  | 9    | CO       | +150 |
///
/// Hand A went to showdown (won 500 sd), saw flop, vpip, one flop aggression.
/// Hand B saw flop, vpip. Hand C saw flop. Hand D vpip. E plain. F showdown
/// loss? (net positive; saw flop, no showdown).
fn standard_pop() -> Vec<Fixture> {
    let t = |d: u32, h: u32| ts_of(2024, 1, d, h);
    let mut a = Fixture::new("A", t(3, 14));
    a.net = 500;
    a.went_to_showdown = true;
    a.saw_flop = true;
    a.vpip = true;
    a.aggressive = [0, 1, 0, 0];

    let mut b = Fixture::new("B", t(3, 19));
    b.position = Position::Btn;
    b.net = -250;
    b.saw_flop = true;
    b.vpip = true;

    let mut c = Fixture::new("C", t(4, 15));
    c.net = 100;
    c.saw_flop = true;

    let mut d = Fixture::new("D", t(4, 20));
    d.profile_id = 2;
    d.position = Position::Btn;
    d.net = 75;
    d.vpip = true;

    let mut e = Fixture::new("E", t(4, 20) + 20 * 60); // 20 min after D: same session
    e.sb = 10;
    e.bb = 25;
    e.table_size = 9;
    e.position = Position::Utg;
    e.net = -25;

    let mut g = Fixture::new("F", t(5, 16));
    g.table_size = 9;
    g.net = 150;
    g.saw_flop = true;
    vec![a, b, c, d, e, g]
}

fn setup_std() -> (Db, texture::storage::tempdir::TempDir, Vec<Fixture>) {
    let (db, dir) = storage::Db::open_temp().unwrap();
    // hero_hands.profile_id has a FK to hero_profiles.id, so the profiles used
    // by the fixtures must exist first.
    db.create_profile("main").unwrap(); // id 1
    db.create_profile("alias").unwrap(); // id 2
    let fixtures = standard_pop();
    for f in &fixtures {
        insert(&db, f);
    }
    (db, dir, fixtures)
}

fn ts(v: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(v, 0).unwrap()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn aggregate_none_is_single_group() {
    let (_db, _dir, _fx) = setup_std();
    let db = &_db;
    let groups = q::aggregate(db, &FilterSpec::default(), GroupBy::None).unwrap();
    assert_eq!(groups.len(), 1);
    let g = &groups[0];
    assert_eq!(g.key, "all");
    let r = &g.row;
    // Lifetime totals over the standard population.
    assert_eq!(r.hands, 6);
    assert_eq!(r.net_result_minor, 500 - 250 + 100 + 75 - 25 + 150); // 550
    assert_eq!(r.adjusted_result_minor, 550);
    assert_eq!(r.luck_delta_minor, 0);
    assert_eq!(r.rake_attributed_minor, 6 * 3);
    assert_eq!(r.showdown_hands, 1);
    assert_eq!(r.sd_winnings_minor, 500);
    assert_eq!(r.non_sd_winnings_minor, 100 + 75 + 150); // won without showdown
    assert_eq!(r.vpip, 3); // A, B, D
    assert_eq!(r.wwsf_opportunity, 4); // saw flop: A, B, C, F
    assert_eq!(r.won_when_saw_flop, 3); // A, C and F won after seeing a flop
    assert_eq!(r.wtsd, 1);
    assert_eq!(r.wtsd_opportunity, 4); // wtsd% is measured of flop-seen hands
}

#[test]
fn empty_filter_equals_unfiltered_lifetime() {
    let (_db, _dir, fx) = setup_std();
    let db = &_db;
    // Explicit "no constraints" vs. `FilterSpec::default()` must be identical,
    // and both must equal the lifetime total (the None-group aggregate).
    let empty_default = q::aggregate(db, &FilterSpec::default(), GroupBy::None).unwrap();
    let empty_struct = FilterSpec::default();
    assert!(empty_struct.is_empty());
    let empty_explicit = q::aggregate(db, &empty_struct, GroupBy::None).unwrap();
    assert_eq!(empty_default.len(), 1);
    assert_eq!(empty_default[0].row.hands, fx.len() as i64);
    assert_eq!(empty_default[0].row.hands, empty_explicit[0].row.hands);
    assert_eq!(
        empty_default[0].row.net_result_minor,
        empty_explicit[0].row.net_result_minor
    );
    assert_eq!(q::hand_count(db, &FilterSpec::default()).unwrap(), fx.len() as i64);
}

#[test]
fn group_by_day_utc_buckets() {
    let (_db, _dir, _fx) = setup_std();
    let db = &_db;
    let groups = q::aggregate(db, &FilterSpec::default(), GroupBy::Day).unwrap();
    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0].key, "2024-01-03");
    assert_eq!(groups[1].key, "2024-01-04");
    assert_eq!(groups[2].key, "2024-01-05");
    assert_eq!(groups[0].row.hands, 2);
    assert_eq!(groups[0].row.net_result_minor, 250);
    assert_eq!(groups[1].row.hands, 3);
    assert_eq!(groups[1].row.net_result_minor, 150);
    assert_eq!(groups[2].row.hands, 1);
    assert_eq!(groups[2].row.net_result_minor, 150);
}

#[test]
fn group_by_stake_position_labels_and_order() {
    let (_db, _dir, _fx) = setup_std();
    let db = &_db;
    let by_stake = q::aggregate(db, &FilterSpec::default(), GroupBy::Stake).unwrap();
    assert_eq!(by_stake.len(), 2);
    // Ordered by stake level (smallest first).
    assert_eq!(by_stake[0].key, "\u{20ac}0.10/\u{20ac}0.25");
    assert_eq!(by_stake[0].row.hands, 1);
    assert_eq!(by_stake[0].row.net_result_minor, -25);
    assert_eq!(by_stake[1].key, "\u{20ac}0.25/\u{20ac}0.50");
    assert_eq!(by_stake[1].row.hands, 5);

    let by_pos = q::aggregate(db, &FilterSpec::default(), GroupBy::Position).unwrap();
    let keys: Vec<&str> = by_pos.iter().map(|g| g.key.as_str()).collect();
    // Ordered SB..BTN.
    assert_eq!(keys, vec!["UTG", "CO", "BTN"]);
    assert_eq!(by_pos[0].row.hands, 1);
    assert_eq!(by_pos[1].row.hands, 3);
    assert_eq!(by_pos[2].row.hands, 2);

    // Week and Month are single buckets here.
    let by_week = q::aggregate(db, &FilterSpec::default(), GroupBy::Week).unwrap();
    assert_eq!(by_week.len(), 1);
    assert_eq!(by_week[0].row.hands, 6);
    let by_month = q::aggregate(db, &FilterSpec::default(), GroupBy::Month).unwrap();
    assert_eq!(by_month.len(), 1);
    assert_eq!(by_month[0].key, "2024-01");
}

#[test]
fn filter_by_from_to_is_inclusive_exclusive() {
    let (_db, _dir, _fx) = setup_std();
    let db = &_db;
    // from = 2024-01-04 00:00Z, to = 2024-01-05 00:00Z → only day 04 hands.
    let filter = FilterSpec {
        from: Some(ts(ts_of(2024, 1, 4, 0))),
        to: Some(ts(ts_of(2024, 1, 5, 0))),
        ..Default::default()
    };
    let groups = q::aggregate(db, &filter, GroupBy::None).unwrap();
    assert_eq!(groups[0].row.hands, 3);
    assert_eq!(groups[0].row.net_result_minor, 100 + 75 - 25);
    assert_eq!(q::hand_count(db, &filter).unwrap(), 3);
}

#[test]
fn filter_by_stakes_pair() {
    let (_db, _dir, _fx) = setup_std();
    let db = &_db;
    let filter = FilterSpec {
        stakes: vec![(10, 25)],
        ..Default::default()
    };
    let groups = q::aggregate(db, &filter, GroupBy::None).unwrap();
    assert_eq!(groups[0].row.hands, 1);
    assert_eq!(groups[0].row.net_result_minor, -25);
}

#[test]
fn filter_by_table_size() {
    let (_db, _dir, _fx) = setup_std();
    let db = &_db;
    let filter = FilterSpec {
        table_sizes: vec![9],
        ..Default::default()
    };
    let groups = q::aggregate(db, &filter, GroupBy::None).unwrap();
    assert_eq!(groups[0].row.hands, 2);
    assert_eq!(groups[0].row.net_result_minor, -25 + 150);
}

#[test]
fn filter_by_position() {
    let (_db, _dir, _fx) = setup_std();
    let db = &_db;
    let filter = FilterSpec {
        positions: vec![Position::Btn],
        ..Default::default()
    };
    let groups = q::aggregate(db, &filter, GroupBy::None).unwrap();
    assert_eq!(groups[0].row.hands, 2);
    assert_eq!(groups[0].row.net_result_minor, -250 + 75);
}

#[test]
fn filter_by_profiles() {
    let (_db, _dir, _fx) = setup_std();
    let db = &_db;
    let filter = FilterSpec {
        profiles: vec![2],
        ..Default::default()
    };
    let groups = q::aggregate(db, &filter, GroupBy::None).unwrap();
    assert_eq!(groups[0].row.hands, 1);
    assert_eq!(groups[0].row.net_result_minor, 75);

    // Both profiles.
    let both = FilterSpec {
        profiles: vec![1, 2],
        ..Default::default()
    };
    assert_eq!(q::hand_count(db, &both).unwrap(), 6);
}

#[test]
fn combined_filters_intersect() {
    let (_db, _dir, _fx) = setup_std();
    let db = &_db;
    // 6-max + €0.25/€0.50 + CO, all days.
    let filter = FilterSpec {
        table_sizes: vec![6],
        stakes: vec![(25, 50)],
        positions: vec![Position::Co],
        ..Default::default()
    };
    let groups = q::aggregate(db, &filter, GroupBy::None).unwrap();
    assert_eq!(groups[0].row.hands, 2); // A and C
    assert_eq!(groups[0].row.net_result_minor, 600);

    // Narrow further with a date window that excludes A.
    let filter2 = FilterSpec {
        from: Some(ts(ts_of(2024, 1, 4, 0))),
        ..filter.clone()
    };
    let groups2 = q::aggregate(db, &filter2, GroupBy::None).unwrap();
    assert_eq!(groups2[0].row.hands, 1); // C only
    assert_eq!(groups2[0].row.net_result_minor, 100);
}

#[test]
fn stake_options_lists_distinct_sorted() {
    let (_db, _dir, _fx) = setup_std();
    let db = &_db;
    let opts = q::stake_options(db).unwrap();
    assert_eq!(opts.len(), 2);
    assert_eq!(opts[0], (10, 25, "\u{20ac}0.10/\u{20ac}0.25".to_string()));
    assert_eq!(opts[1], (25, 50, "\u{20ac}0.25/\u{20ac}0.50".to_string()));
}

#[test]
fn bb_values_per_hand_computed() {
    let (_db, _dir, _fx) = setup_std();
    let db = &_db;
    // Just hands A and B: net +500 (10bb) and -250 (5bb) at the same stake.
    let filter = FilterSpec {
        from: Some(ts(ts_of(2024, 1, 3, 0))),
        to: Some(ts(ts_of(2024, 1, 4, 0))),
        ..Default::default()
    };
    let groups = q::aggregate(db, &filter, GroupBy::None).unwrap();
    let r = &groups[0].row;
    assert_eq!(r.hands, 2);
    // bb_won = 10 + (-5) = 5.0 big blinds.
    assert!((r.bb_won - 5.0).abs() < 1e-9);
    // bb_per_100 = 5.0 / 2 * 100 = 250.
    assert!((r.bb_per_100 - 250.0).abs() < 1e-9);
    // currency_per_100 (docs/stat-definitions.md): Σ net / hands × 100 in
    // minor units = (500-250)/2*100 = 12500 minor  (displayed as €125.00/100).
    assert!((r.currency_per_100 - 12500.0).abs() < 1e-9);
    // total_big_blinds = Σ bb_minor / 100 = (50*2)/100 = 1.0.
    assert!((r.total_big_blinds - 1.0).abs() < 1e-9);
}

/// Mixed stakes: bb_won must weight each hand by its own big blind.
#[test]
fn bb_won_mixed_stakes_uses_per_hand_bb() {
    let (_db, _dir, _fx) = setup_std();
    let db = &_db;
    // Hand E: net -25 at bb 25 → -1 bb. Hand F: net +150 at bb 50 → +3 bb.
    let filter = FilterSpec {
        table_sizes: vec![9],
        ..Default::default()
    };
    let groups = q::aggregate(db, &filter, GroupBy::None).unwrap();
    let r = &groups[0].row;
    assert_eq!(r.hands, 2);
    assert!((r.bb_won - (-1.0 + 3.0)).abs() < 1e-9);
    assert!((r.bb_per_100 - ((-1.0 + 3.0) / 2.0 * 100.0)).abs() < 1e-9);
    // total_big_blinds = (25 + 50)/100 = 0.75.
    assert!((r.total_big_blinds - 0.75).abs() < 1e-9);
}

#[test]
fn street_indexed_arrays_fold_by_offset() {
    let (db, _dir) = setup_simple();
    let db = &db;
    // One hand with cbet_opportunity [flop=true, turn=false, river=true],
    // cbet [f, t, r] = [1, 1, 0], aggressive [2, 3, 4, 5], passive [1, 0, 1, 2].
    let mut f = Fixture::new("X", ts_of(2024, 2, 1, 10));
    f.vpip = true;
    let mut a = build_analysis(&f);
    a.cbet_opportunity = [true, false, true];
    a.cbet = [true, true, false];
    a.check_raise_opportunity = [false, true, false];
    a.check_raise = [false, true, false];
    a.aggressive_actions = [2, 3, 4, 5];
    a.passive_actions = [1, 0, 1, 2];
    a.player_id = db.upsert_player(HERO).unwrap();
    let parsed = build_parsed(&f);
    hands::upsert_hand(&db, &parsed, Some(&a), "raw", "t.txt").unwrap();

    let groups = q::aggregate(db, &FilterSpec::default(), GroupBy::None).unwrap();
    let r = &groups[0].row;
    assert_eq!(r.cbet_opportunity, [1, 0, 1]);
    assert_eq!(r.cbet, [1, 1, 0]);
    assert_eq!(r.check_raise_opportunity, [0, 1, 0]);
    assert_eq!(r.check_raise, [0, 1, 0]);
    assert_eq!(r.aggressive_actions, [2, 3, 4, 5]);
    assert_eq!(r.passive_actions, [1, 0, 1, 2]);
}

fn setup_simple() -> (Db, texture::storage::tempdir::TempDir) {
    let (db, dir) = storage::Db::open_temp().unwrap();
    db.create_profile("main").unwrap(); // id 1, needed for the hero_hands FK
    (db, dir)
}

#[test]
fn hand_count_and_list_respect_filters_and_paging() {
    let (_db, _dir, _fx) = setup_std();
    let db = &_db;
    let filter = FilterSpec {
        positions: vec![Position::Co],
        ..Default::default()
    };
    assert_eq!(q::hand_count(db, &filter).unwrap(), 3);

    let rows = q::hand_list(db, &filter, 2, 0).unwrap();
    assert_eq!(rows.len(), 2);
    // Newest first.
    assert_eq!(rows[0].hand_id, "F");
    assert_eq!(rows[0].position, "CO");
    assert_eq!(rows[0].stakes, "\u{20ac}0.25/\u{20ac}0.50");
    assert_eq!(rows[0].currency, "EUR");
    assert_eq!(rows[0].net_result_minor, 150);
    assert_eq!(rows[0].table_size, 9);
    let page2 = q::hand_list(db, &filter, 2, 2).unwrap();
    assert_eq!(page2.len(), 1);
    assert_eq!(page2[0].hand_id, "A");

    // Nothing matches → empty.
    let none = FilterSpec {
        positions: vec![Position::Sb],
        ..Default::default()
    };
    assert_eq!(q::hand_count(db, &none).unwrap(), 0);
    assert!(q::hand_list(db, &none, 10, 0).unwrap().is_empty());
}

#[test]
fn cumulative_series_small_population_returns_all_points() {
    let (_db, _dir, fx) = setup_std();
    let db = &_db;
    let pts = q::cumulative_series(db, &FilterSpec::default(), 100).unwrap();
    assert_eq!(pts.len(), fx.len());
    // Ordered by ts: A(+500), B(-250), C(+100), D(+75), E(-25), F(+150).
    assert_eq!(pts[0].net_minor, 500);
    assert_eq!(pts[0].adjusted_minor, 500);
    assert_eq!(pts[1].net_minor, 250);
    assert_eq!(pts[2].net_minor, 350);
    assert_eq!(pts[3].net_minor, 425);
    assert_eq!(pts[4].net_minor, 400);
    assert_eq!(pts[5].net_minor, 550);
    assert_eq!(pts[0].t, ts_of(2024, 1, 3, 14));
    assert_eq!(pts[0].hands, 1);
    assert_eq!(pts[5].hands, 6);
}

#[test]
fn cumulative_series_boundary_dates_filter() {
    let (_db, _dir, _fx) = setup_std();
    let db = &_db;
    let filter = FilterSpec {
        from: Some(ts(ts_of(2024, 1, 4, 0))),
        ..Default::default()
    };
    let pts = q::cumulative_series(db, &filter, 100).unwrap();
    assert_eq!(pts.len(), 4);
    assert_eq!(pts[0].net_minor, 100);
    assert_eq!(pts[3].net_minor, 300); // 100+75-25+150
}

#[test]
fn cumulative_series_downsample_bounds_and_endpoints() {
    // 400 hands, alternating +50/-30 nets so the cumulative curve oscillates;
    // downsample to at most 30 points.
    let (db, _dir) = setup_simple();
    let db = &db;
    let base = ts_of(2024, 3, 1, 0);
    for i in 0..400 {
        let net = if i % 2 == 0 { 50 } else { -30 };
        let f = Fixture {
            hand_id: format!("S{i:05}"),
            ts: base + i * 10, // 10s apart, no session splits (gap = 30 min)
            profile_id: 1,
            position: Position::Co,
            table_size: 6,
            sb: 25,
            bb: 50,
            net,
            went_to_showdown: false,
            saw_flop: false,
            vpip: false,
            aggressive: [0; 4],
        };
        insert(&db, &f);
    }
    let max_points = 30;
    let pts = q::cumulative_series(&db, &FilterSpec::default(), max_points).unwrap();
    assert!(pts.len() <= max_points);
    // Monotonic non-decreasing hands count (points stay ordered).
    assert!(pts.windows(2).all(|w| w[0].hands < w[1].hands));
    // First point is the first hand; last point is the lifetime total.
    assert_eq!(pts[0].hands, 1);
    assert_eq!(pts[0].net_minor, 50);
    let last = pts.last().unwrap();
    let expected_total: i64 = (0..400).map(|i| if i % 2 == 0 { 50 } else { -30 }).sum();
    assert_eq!(last.net_minor, expected_total);
    assert_eq!(last.hands, 400);
    // And the unfiltered cumulative equals the sum along the way.
    assert_eq!(last.adjusted_minor, expected_total);
}

#[test]
fn session_filter_keeps_only_wanted_sessions() {
    let (_db, _dir, _fx) = setup_std();
    let db = &_db;
    // Sessions over the standard pop (default gap = 30 min):
    //   A(3rd 14:00),B(3rd 19:00): 5h gap → sessions 0, 1.
    //   C(4th 15:00): 20h gap → session 2.
    //   D(4th 20:00): 5h gap → session 3.
    //   E(4th 20:20): 20 min gap → same session 3 as D.
    //   F(5th 16:00): ~20h gap → session 4.
    assert_eq!(db.session_gap_secs().unwrap(), 30 * 60);
    let s0 = FilterSpec {
        sessions: vec![0],
        ..Default::default()
    };
    let groups = q::aggregate(db, &s0, GroupBy::None).unwrap();
    assert_eq!(groups[0].row.hands, 1);
    assert_eq!(groups[0].row.net_result_minor, 500); // hand A

    let s3 = FilterSpec {
        sessions: vec![3],
        ..Default::default()
    };
    let groups = q::aggregate(db, &s3, GroupBy::None).unwrap();
    assert_eq!(groups[0].row.hands, 2); // D + E
    assert_eq!(groups[0].row.net_result_minor, 75 - 25);

    // Two sessions combined.
    let s13 = FilterSpec {
        sessions: vec![1, 3],
        ..Default::default()
    };
    assert_eq!(q::hand_count(db, &s13).unwrap(), 3);
    assert_eq!(q::hand_count(db, &FilterSpec::default()).unwrap(), 6);
}

#[test]
fn session_ids_recalc_when_the_population_changes() {
    let (_db, _dir, _fx) = setup_std();
    let db = &_db;
    // With the BTN filter the ordered population is B(3rd 19:00),
    // D(4th 20:00), F(5th 16:00) — three distinct 21h+ gaps → three sessions,
    // so session 1 is now D, not B.
    let filter = FilterSpec {
        positions: vec![Position::Btn],
        sessions: vec![1],
        ..Default::default()
    };
    let groups = q::aggregate(db, &filter, GroupBy::None).unwrap();
    assert_eq!(groups[0].row.hands, 1);
    assert_eq!(groups[0].row.net_result_minor, 75); // hand D under this population
}

#[test]
fn empty_db_yields_empty_results() {
    let (db, _dir) = setup_simple();
    let db = &db;
    assert!(q::aggregate(db, &FilterSpec::default(), GroupBy::None)
        .unwrap()
        .is_empty());
    assert!(q::cumulative_series(db, &FilterSpec::default(), 10).unwrap().is_empty());
    assert_eq!(q::hand_count(db, &FilterSpec::default()).unwrap(), 0);
    assert!(q::hand_list(db, &FilterSpec::default(), 10, 0).unwrap().is_empty());
    assert!(q::stake_options(db).unwrap().is_empty());
}

#[test]
fn hand_list_orders_descending() {
    let (_db, _dir, _fx) = setup_std();
    let db = &_db;
    let rows = q::hand_list(db, &FilterSpec::default(), 6, 0).unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r.hand_id.as_str()).collect();
    // Newest first; ties broken by hand id ascending.
    assert_eq!(ids, vec!["F", "E", "D", "C", "B", "A"]);
    assert!(rows.windows(2).all(|w| w[0].ts_utc >= w[1].ts_utc));
}
