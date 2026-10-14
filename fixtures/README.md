# Parser fixture corpus

## These fixtures are synthesized

No real PokerStars hand histories were available on the development machine (the
client purges old histories, and nothing was found on disk). Every `.txt` here
was therefore **written by hand** to match the documented PokerStars format,
rather than captured from play.

What that means:

- The corpus proves the parser is self-consistent and covers the edge cases the
  spec enumerates. It does **not** prove the parser matches a real PokerStars
  client byte for byte.
- Every format decision that a real file could contradict is listed under
  *Assumptions* below. The first real import is the real test; anything that
  differs should become a new fixture (and a parser fix), not a silent change.
- The arithmetic in every fixture is deliberately exact, because the parser
  enforces it: contributions (excluding returned uncalled bets) must equal
  `Total pot`, rake must not exceed the pot, and no seat may collect more than
  the pot. `tests/parser_golden.rs` re-checks all of these for every fixture.

## Corpus

| Fixture | Covers |
|---|---|
| `fold_to_bb.txt` | Everyone folds to the big blind; uncalled blind returned |
| `no_showdown.txt` | Hand ends with a river fold; five-card board; no showdown |
| `limped_multiway.txt` | Limped multiway pot reaching showdown; a muck |
| `split_pot.txt` | Split pot; two `collected` lines; the board plays |
| `allin_preflop_twoway.txt` | Two-way all-in preflop with a known run-out |
| `allin_threeway_sidepot.txt` | Three-way all-in with a short stack and a side pot |
| `short_allin_for_less.txt` | A player calls all-in for less than the bet |
| `posted_dead_blind.txt` | A returning player posts a dead blind |
| `timeout_and_sitout.txt` | `is sitting out` seat plus a `has timed out` notice |
| `nine_max_full_ring.txt` | A 9-max table reaching the river |
| `check_raise.txt` | Check, face a bet, raise |
| `four_bet_pot.txt` | Open, 3-bet, 4-bet, then a flop fold |

Each `.txt` has a matching `.json` golden. Regenerate them deliberately:

```
cargo test --test parser_golden -- --ignored regenerate_goldens
```

Then **review the diff**. The goldens are a regression tripwire; the
hand-authored expectations in `tests/parser_golden.rs` are the actual contract.

## Assumptions

These are the format decisions the parser makes that a real file could
contradict. Each is recorded here rather than left implicit.

1. **Dead blinds.** Written as a standalone `NAME: posts dead blind €0.50` line,
   posted in addition to the normal blinds. Real PokerStars files are believed to
   attach dead-blind wording to a blind post instead; the parser also accepts
   `posts big blind … + dead blind`, but this specific form is unverified.
2. **Timeout notices.** `NAME has timed out` (no colon) is treated as a notice,
   not an action. `is disconnected` and `has returned` are likewise ignored.
3. **Currency notation.** Only EUR is accepted, in either `€0.25/€0.50 EUR` or
   symbol-only form. A `$` hand fails with `UnsupportedCurrency` — deliberately,
   because v1 assumes a single EUR account and mixing currencies would corrupt
   every total. There is a unit test for this in the parser.
4. **Timestamps.** The bracketed Eastern-time stamp is the UTC anchor, using US
   Eastern rules (EST = UTC−5, EDT = UTC−4, DST from the second Sunday in March
   to the first Sunday in November). The unbracketed stamp and its timezone token
   are preserved verbatim. If PokerStars ever writes a different bracketed
   timezone, this is wrong.
5. **Jackpot drop.** A separately reported `Jackpot`/`Jackpot drop` amount is
   parsed into `jackpot_drop` and accepted as either inside or outside
   `Total pot`. No fixture exercises it, because it was not observed in the
   player's games — this is the one summary line the corpus does not cover.
6. **Position labels.** 6-max is `SB, BB, UTG, MP, CO, BTN`; 9-max is
   `SB, BB, UTG, UTG1, UTG2, MP, HJ, CO, BTN`. Heads-up labels the button `BTN`
   and the other seat `BB` (the button posts the small blind), so there is no
   `SB` row at 2-max.
7. **`streets_dealt`** always begins with `Preflop`.
8. **`shown_cards`** holds the hero's dealt hole cards (from `Dealt to`) plus any
   cards an opponent revealed with `shows`. This is what the EV engine needs, and
   it is why `shown_cards` alone cannot identify which seat is the hero.
9. **Collection lines** are `NAME collected €X from pot` with no colon, and are
   summed when a pot is split across a main and a side pot.
10. **Raises.** `raises €X to €Y` is read as "the seat's total commitment on this
    street becomes Y", so `Action::amount` is the chips actually moved, which is
    not the same as the written `X` when the seat already had a blind posted.

## Adding real files

Drop real histories into a directory and import them:

```
TEXTURE_HERO_NAME=<your screen name> texture import /path/to/HandHistory
```

Any hand that fails to parse is quarantined with a reason and shown on the
import screen; it is never silently dropped. Promote each new odd hand into this
corpus as a fixture.
