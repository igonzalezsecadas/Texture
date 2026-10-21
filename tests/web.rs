//! Web layer tests. These drive the real router over a real database, so a
//! template that fails to render, a filter that never reaches the query, or a
//! statistic with no definition all fail here rather than in the browser.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::path::Path;
use std::sync::Arc;
use texture::config::Config;
use texture::importer::{self, ImportOptions, ImportProgress};
use texture::storage::{self, Db};
use texture::web::{self, AppState};
use tower::ServiceExt;

/// One complete hand where the hero (seat 2, the big blind) takes the pot down
/// preflop. Identical in shape to the end-to-end acceptance fixture.
fn walk_hand(hand_id: u64, hero: &str, day: u32) -> String {
    format!(
        r#"PokerStars Hand #{hand_id}:  Hold'em No Limit (€0.25/€0.50 EUR) - 2024/01/{day:02} 13:00:00 CET [2024/01/{day:02} 7:00:00 ET]
Table 'Alpha' 6-max Seat #6 is the button
Seat 1: villainAA (€50 in chips)
Seat 2: {hero} (€50 in chips)
Seat 3: villainBB (€50 in chips)
Seat 4: villainCC (€50 in chips)
Seat 5: villainDD (€50 in chips)
Seat 6: villainEE (€50 in chips)
villainAA: posts small blind €0.25
{hero}: posts big blind €0.50
*** HOLE CARDS ***
Dealt to {hero} [As Kd]
villainBB: folds
villainCC: folds
villainDD: folds
villainEE: folds
villainAA: folds
Uncalled bet (€0.25) returned to {hero}
{hero} collected €0.50 from pot
*** SUMMARY ***
Total pot €0.50 | Rake €0.00
Seat 1: villainAA (small blind) folded before Flop
Seat 2: {hero} (big blind) collected (€0.50)
"#
    )
}

struct Env {
    state: AppState,
    db: Db,
    _hands: tempfile::TempDir,
    _data: tempfile::TempDir,
}

fn config_for(data: &Path, hands: &Path) -> Config {
    Config {
        hand_history_dir: hands.to_path_buf(),
        data_dir: data.to_path_buf(),
        snapshot_dir: data.join("snapshots"),
        bind_addr: "127.0.0.1:0".parse().expect("addr"),
        snapshot_interval_secs: 0,
        snapshot_retention: 3,
        hero_name: None,
        scan_interval_secs: 30,
        log_json: false,
    }
}

/// A populated database: three imported hands for a configured hero.
fn populated() -> Env {
    let hands = tempfile::tempdir().expect("hands dir");
    let data = tempfile::tempdir().expect("data dir");
    let cfg = config_for(data.path(), hands.path());
    let db = Db::open(&cfg.db_path()).expect("open db");
    db.ensure_hero_profile("hero").expect("hero profile");

    let mut text = String::new();
    for (i, day) in [2u32, 3, 4].into_iter().enumerate() {
        text.push_str(&walk_hand(1000 + i as u64, "hero", day));
        text.push('\n');
    }
    std::fs::write(hands.path().join("2024-01.txt"), text).expect("write hands");

    let opts = ImportOptions::default();
    let mut progress = |_p: &ImportProgress| {};
    let outcome = importer::import_directory(&db, hands.path(), &opts, &mut progress)
        .expect("import succeeds");
    assert_eq!(outcome.hands_imported, 3, "fixture import must land three hands");
    assert_eq!(outcome.quarantined, 0);

    let state = AppState {
        db: db.clone(),
        cfg: Arc::new(cfg),
        importer: importer::shared_importer_handle().clone(),
    };
    Env { state, db, _hands: hands, _data: data }
}

/// An empty database with no hero configured and nothing imported.
fn empty() -> Env {
    let hands = tempfile::tempdir().expect("hands dir");
    let data = tempfile::tempdir().expect("data dir");
    let cfg = config_for(data.path(), hands.path());
    let db = Db::open(&cfg.db_path()).expect("open db");
    let state = AppState {
        db: db.clone(),
        cfg: Arc::new(cfg),
        importer: importer::shared_importer_handle().clone(),
    };
    Env { state, db, _hands: hands, _data: data }
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, String) {
    let app = web::router(state.clone());
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).expect("request"))
        .await
        .expect("router responds");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).to_string())
}

const SCREENS: [&str; 8] = [
    "/", "/preflop", "/postflop", "/position", "/sessions", "/hands", "/import", "/settings",
];

#[tokio::test]
async fn every_screen_renders_on_a_populated_database() {
    let env = populated();
    for screen in SCREENS {
        let (status, body) = get(&env.state, screen).await;
        assert_eq!(status, StatusCode::OK, "{screen} should render: {body}");
        assert!(!body.trim().is_empty(), "{screen} rendered an empty body");
        // The shared layout, and therefore the filter bar, is on every report.
        if screen != "/import" && screen != "/settings" {
            assert!(
                body.contains("active-filters"),
                "{screen} must carry the filter bar's active state"
            );
        }
    }
}

#[tokio::test]
async fn every_screen_renders_on_an_empty_database() {
    let env = empty();
    for screen in SCREENS {
        let (status, body) = get(&env.state, screen).await;
        assert_eq!(status, StatusCode::OK, "{screen} must render with no data: {body}");
    }
}

#[tokio::test]
async fn healthz_reports_status_without_touching_write_state() {
    let env = populated();
    let (status, body) = get(&env.state, "/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("ok"), "healthz body: {body}");
    assert!(body.contains("version"), "healthz body: {body}");
    assert!(body.contains("3"), "healthz should report the hand count: {body}");
}

#[tokio::test]
async fn dashboard_shows_the_results_and_the_chart_payload() {
    let env = populated();
    let (_, body) = get(&env.state, "/").await;
    assert!(body.contains("Total won"), "the dashboard must show a results summary");
    assert!(body.contains("All-in adjusted"), "the EV line must be present");
    // Three winning walks of 25 minor units each.
    assert!(body.contains("€0.75"), "the net result should be three walks: {body}");
    assert!(body.contains("chart-data"), "the chart payload must be embedded");
    assert!(body.contains("uPlot"), "the vendored charting library must be referenced");
}

#[tokio::test]
async fn a_filter_reaches_the_query_and_is_visible() {
    let env = populated();
    let (_, unfiltered) = get(&env.state, "/hands").await;
    let (_, filtered) = get(&env.state, "/hands?from=2999-01-01").await;
    assert!(
        unfiltered.contains("2024-01-02"),
        "the unfiltered hand list should contain the imported hands"
    );
    assert!(
        !filtered.contains("2024-01-02"),
        "a future date range must select nothing, so the filter reached the query"
    );
    assert!(
        filtered.contains("Clear all"),
        "an active filter set must be visible and clearable"
    );
    assert!(
        unfiltered.contains("lifetime totals"),
        "an empty filter set must present itself as the lifetime total"
    );
}

#[tokio::test]
async fn every_statistic_shown_carries_its_definition() {
    let env = populated();
    for screen in ["/preflop", "/postflop", "/position"] {
        let (status, body) = get(&env.state, screen).await;
        assert_eq!(status, StatusCode::OK);
        // Definitions are rendered as `title="..."` on an `abbr.def`.
        let titles: Vec<&str> = body
            .split("title=\"")
            .skip(1)
            .filter_map(|chunk| chunk.split('"').next())
            .collect();
        assert!(!titles.is_empty(), "{screen} must show statistics");
        // The nav's own title attributes are short; a real definition is a
        // sentence, so require every abbreviation to carry actual prose.
        let empty = titles.iter().filter(|t| t.len() < 15).count();
        assert!(
            empty <= 2,
            "{screen} has {empty} statistics without a dictionary definition: {titles:?}"
        );
        assert!(
            titles.iter().any(|t| t.contains("voluntarily") || t.contains("hero") || t.contains("aggressor")),
            "{screen} definitions do not look like dictionary prose: {titles:?}"
        );
    }
}

#[tokio::test]
async fn hand_detail_shows_the_parse_and_the_original_text() {
    let env = populated();
    let listed = storage::queries::hand_list(&env.db, &Default::default(), 5, 0).expect("hand list");
    assert_eq!(listed.len(), 3);
    let id = &listed[0].hand_id;

    let (status, body) = get(&env.state, &format!("/hands/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("PokerStars Hand #"), "the raw text must be shown verbatim");
    assert!(body.contains("Original hand text"), "the raw panel must be labelled");
    assert!(body.contains("Parsed"), "the parsed panel must be present");
    assert!(body.contains("big blind"), "positions from the parse must be shown");

    let (missing, _) = get(&env.state, "/hands/999999").await;
    assert_eq!(missing, StatusCode::NOT_FOUND, "an unknown hand is a 404");
}

#[tokio::test]
async fn csv_exports_are_downloadable() {
    let env = populated();
    let (status, body) = get(&env.state, "/export/aggregate.csv").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.lines().count() >= 2, "aggregate csv needs a header and a row: {body}");

    let (status, body) = get(&env.state, "/export/hands.csv").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("hand_id"), "hands csv header: {body}");
    assert!(body.contains("PokerStars Hand #"), "hands csv must carry the raw text");
}

#[tokio::test]
async fn sessions_are_listed_and_labelled_approximate() {
    let env = populated();
    let (status, body) = get(&env.state, "/sessions").await;
    assert_eq!(status, StatusCode::OK);
    // The three hands are a day apart, so the default 30 minute gap makes three
    // sessions.
    assert!(body.contains("2024-01-02"), "sessions must list their start: {body}");
    assert!(body.contains("approx"), "durations must be labelled approximate");
}

#[tokio::test]
async fn import_screen_reports_status_and_quarantine() {
    let env = populated();
    let (status, body) = get(&env.state, "/import").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Quarantined"), "the quarantine count must be surfaced");
    assert!(body.contains("Hands imported"), "the importer status must be shown");
}

#[tokio::test]
async fn settings_lists_the_hero_profile_and_its_aliases() {
    let env = populated();
    let (status, body) = get(&env.state, "/settings").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("hero"), "the screen name must be listed");
    assert!(body.contains("Add alias"), "renames must be manageable");
    assert!(body.contains("snapshot"), "a manual snapshot must be reachable");
}

#[tokio::test]
async fn static_assets_are_served_from_the_embedded_bundle() {
    let env = empty();
    for (path, needle) in [
        ("/static/app.css", "filter"),
        ("/static/app.js", "uPlot"),
        ("/static/htmx.min.js", "htmx"),
        ("/static/uPlot.iife.min.js", "uPlot"),
        ("/static/uPlot.min.css", "uplot"),
    ] {
        let (status, body) = get(&env.state, path).await;
        assert_eq!(status, StatusCode::OK, "{path} must be served");
        assert!(body.contains(needle), "{path} looks wrong");
    }
    let (missing, _) = get(&env.state, "/static/nope.js").await;
    assert_eq!(missing, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_session_links_to_its_own_hands() {
    let env = populated();
    let (_, sessions) = get(&env.state, "/sessions").await;
    assert!(sessions.contains("review hands"), "each session must be reviewable");
    // Session 0 is the first hand only; the link must carry the session filter.
    // The URL is HTML-escaped by the template (minijinja escapes `/`), which a
    // browser decodes, so assert on the part that identifies the filter.
    assert!(
        sessions.contains("href=") && sessions.contains("session=0"),
        "sessions must link into the browser: {sessions}"
    );

    let (status, hands) = get(&env.state, "/hands?session=0").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        hands.contains("Clear all"),
        "the session filter must be visible on the hand browser"
    );
    // Exactly one of the three hands belongs to session 0.
    let listed = hands.matches("2024-01-").count();
    assert!(listed > 0, "the session's hands must be listed");
    let (_, all) = get(&env.state, "/hands").await;
    assert!(
        all.matches("2024-01-").count() > listed,
        "a session filter must select fewer hands than the unfiltered list"
    );
}

// ---------------------------------------------------------------------------
// POST /import/upload — browser hand-history uploads
// ---------------------------------------------------------------------------

/// Build a multipart/form-data body. `fields` is `(name, filename, text)`;
/// a `None` filename makes it a plain text field (the paste box).
fn multipart_body(fields: &[( &str, Option<&str>, &str)]) -> (String, Vec<u8>) {
    const BOUNDARY: &str = "texture-test-boundary";
    let mut body = Vec::new();
    for (name, filename, text) in fields {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        match filename {
            Some(file) => body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{name}\"; filename=\"{file}\"\r\n\
                     Content-Type: text/plain\r\n\r\n"
                )
                .as_bytes(),
            ),
            None => body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
            ),
        }
        body.extend_from_slice(text.as_bytes());
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={BOUNDARY}"), body)
}

async fn upload(state: &AppState, fields: &[(&str, Option<&str>, &str)]) -> (StatusCode, String) {
    let (content_type, body) = multipart_body(fields);
    let app = web::router(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/import/upload")
                .header("content-type", content_type)
                .body(Body::from(body))
                .expect("upload request"),
        )
        .await
        .expect("router responds");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).to_string())
}

fn db_hand_count(db: &Db) -> i64 {
    db.with_reader(|c| c.query_row("SELECT COUNT(*) FROM hands", [], |r| r.get(0)))
        .expect("count hands")
}

#[tokio::test]
async fn upload_imports_new_hands_once_and_reports_duplicates() {
    let env = populated();
    let before = db_hand_count(&env.db);
    let text = format!(
        "{}\n{}",
        walk_hand(3000, "hero", 10),
        walk_hand(3001, "hero", 11)
    );

    let (status, body) =
        upload(&env.state, &[("files", Some("session.txt"), text.as_str())]).await;
    assert_eq!(status, StatusCode::OK, "upload should render the import screen: {body}");
    assert!(body.contains("2 new hands added"), "the new-hand summary must be shown: {body}");
    assert!(body.contains("session.txt"), "the source must be named");
    assert_eq!(db_hand_count(&env.db), before + 2);

    let (status, body) =
        upload(&env.state, &[("files", Some("session.txt"), text.as_str())]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("Nothing new") && body.contains("already here"),
        "a re-upload must report duplicates rather than inserting rows: {body}"
    );
    assert_eq!(db_hand_count(&env.db), before + 2, "the hand table must not grow");
}

#[tokio::test]
async fn upload_accepts_pasted_text_and_reports_parse_failures() {
    let env = populated();
    let before = db_hand_count(&env.db);
    let text = format!(
        "{}\nPokerStars Hand #9998: not a real hand\n*** SUMMARY ***\nTotal pot €0.00\n",
        walk_hand(3002, "hero", 12)
    );

    let (status, body) = upload(&env.state, &[("pasted", None, text.as_str())]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("pasted text"), "the paste source must be named: {body}");
    assert!(body.contains("1 new hand added"), "the good hand must land: {body}");
    assert!(body.contains("Quarantined"), "the failure count must be surfaced: {body}");
    assert_eq!(db_hand_count(&env.db), before + 1);
    assert_eq!(
        storage::hands::quarantine_count(&env.db).expect("quarantine count"),
        1,
        "a malformed uploaded hand must be quarantined, never dropped"
    );
}

#[tokio::test]
async fn a_first_upload_registers_the_hero_from_the_hands() {
    let env = empty();
    assert!(env.db.active_profile().expect("profile lookup").is_none());

    let text = format!("{}\n{}", walk_hand(4000, "hero", 2), walk_hand(4001, "hero", 3));
    let (status, body) =
        upload(&env.state, &[("files", Some("HH.txt"), text.as_str())]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("Registered") && body.contains("hero"),
        "the upload must resolve the hero without a settings visit: {body}"
    );

    let profile = env.db.active_profile().expect("profile lookup").expect("a hero exists");
    assert_eq!(profile.name, "default");
    let analyses: i64 = env
        .db
        .with_reader(|c| c.query_row("SELECT COUNT(*) FROM hero_hands", [], |r| r.get(0)))
        .expect("count hero hands");
    assert_eq!(analyses, 2, "first-upload hands must be attributed to the detected hero");
}

#[tokio::test]
async fn an_empty_upload_is_a_validation_error_not_a_crash() {
    let env = empty();
    let (status, body) = upload(&env.state, &[("files", Some("empty.txt"), "   \n\n")]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("Choose a hand-history file") || body.contains("No complete hands found"),
        "an empty upload should explain itself: {body}"
    );
    assert_eq!(db_hand_count(&env.db), 0);
}

#[tokio::test]
async fn the_filter_form_uses_the_parameter_names_the_parser_reads() {
    let env = populated();
    let (_, body) = get(&env.state, "/hands").await;
    assert!(body.contains("name=\"stake\""), "the stake selector must post `stake`");
    assert!(body.contains("name=\"pos\""), "the position selector must post `pos`");
    assert!(
        !body.contains("name=\"stakes\""),
        "`stakes` is not a query parameter the parser understands"
    );

    // The fixture hands are €0.25/€0.50, so this stake selects them and a
    // different stake selects nothing. This is the regression the old form's
    // mismatched parameter name hid.
    let (_, matching) = get(&env.state, "/hands?stake=25-50").await;
    assert!(matching.contains("2024-01-02"), "the matching stake must select the hands");
    let (_, other) = get(&env.state, "/hands?stake=9999-9999").await;
    assert!(!other.contains("2024-01-02"), "a different stake must select nothing");
    assert!(other.contains("Clear all"), "the stake filter must be visible as a chip");
}
