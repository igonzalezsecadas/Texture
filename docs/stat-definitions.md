# Statistic dictionary

Status: authoritative. The stats module (`src/stats`) implements exactly this document.
Reference semantics: PokerTracker 4 / Hold'em Manager 3 ("PT4/HM3"). Field names in
formulas are the frozen contract in `src/domain.rs` (`HeroHandAnalysis`, `ParsedHand`).

Every report draws its numbers by **summing per-hand counter fields over the filtered
population** (`FilterSpec`) and deriving the ratio at presentation time. Percentages are
never averaged as percentages — a stat is always `sum(numerator) / sum(denominator)`
over the same rows its neighbors use. One hand contributes at most one numerator unit
and one denominator unit per statistic; per-hand booleans are 0/1 counts.

## Conventions used by every definition

- **Dealt in**: the hero holds cards at the start of the hand. This is the base
  reporting unit (spec story 23: only hands the hero was dealt into are counted).
- **Decision point**: the hero is still live (has not folded, has cards, is not
  all-in) and a voluntary action (fold / check / call / bet / raise) is available to
  the hero. A player who is all-in has no further decision points.
- **Live opponent**: a player still holding cards whose hand can win any part of the
  pot (in a side pot or the main pot). An opponent who is **all-in counts as live**
  for pot-content purposes but grants no *acting* opportunity to anyone.
- **Acting opportunity** (per street): the hero has a decision point on this street
  and at least one live opponent is able to act (has chips behind, has not folded).
  If every live opponent is all-in, the hero's "bet" can never be called; the street
  grants **no acting opportunity** for bet/raise statistics.
- **Preflop aggressor**: the player who made the last raise (or all-in raise)
  preflop. There is exactly one per hand unless the pot was never raised (limped or
  blind-vs-blind-checked pots have no aggressor).
- **Uncalled bet**: the excess of the final aggressive action over the largest call
  is returned to the aggressor before the pot is awarded. It is not a contribution,
  is not in the pot, and did not happen for postflop attribution purposes — but the
  aggressive action itself (an uncalled raise) still counts as that action
  (an uncalled open still counts as RFI/3-bet/4-bet/steal attempt).
- **Won a pot**: `won_amount > contributed` for the hand — the seat collected more
  chips from the pot than it put in. Used by WWSF/W$SD. Net-of-rake equivalence:
  `"won a pot"` and `net_result > 0` differ only in the razor-edge case where the
  rake equals the entire profit of the win; we use the pot-based definition
  (see Ambiguity log A6).
- **Money**: integer minor units; EUR has 2 decimals (exponent 2), so 1 minor unit
  = €0.01. No floating-point money anywhere.
- **Big blind per hand**: every BB-normalized statistic uses the hand's own
  `big_blind` (per-hand normalization), never one nominal BB for a multi-stake
  filter. For cross-stake views, group by stake (story 63) to read a clean rate.
- **Streets**: `Preflop, Flop, Turn, River` (`Street::ALL`). Postflop arrays are
  indexed `[flop, turn, river]` = indices 0..2; aggression arrays are indexed
  `[preflop, flop, turn, river]` = indices 0..3, exactly as in `HeroHandAnalysis`.

---

## General statistics

### Hands played
**Definition:** count of `HeroHandAnalysis` rows (one per dealt-in hand) matching the
filter — i.e. `COUNT(*)` over the analysis table.
**Opportunity:** the hero was dealt in. Hands where the hero merely appears on the
seats list but sat out or was not dealt cards are excluded.
**Edge cases:** the hand end before the hero's first action still counts (e.g. the
action folds to the hero in the BB and the BB wins uncontested — the hero was dealt
in and was next to act). Dead-blind posts do not create a hand: being dealt in does.
Sessions, dates, stakes, table size, position, and alias filters all shrink this
number together with every other statistic on the page (story 38).

### Net result (winnings)
**Definition:** `Σ net_result` over filtered rows, in minor units, displayed as
`€X.YZ` with the row currency code (v1: EUR only).
**Opportunity:** none — this is an outcome, not a rate. It is reported for every
dealt-in hand, including total losses and folded-preflop blinds.
**Edge cases:** net result is the hero's realized net cash change for the hand
**including rake paid** (rake is a cost of winning, exactly as in PT4 "Net Won").
Uncalled-bet returns are already excluded because `parsed_seat.contributed`
excludes them and `result = won_amount − contributed`. Split pots contribute each
winners' share; jackpot drops are never deducted from net result (only rake is).

### BB/100
**Definition:** `(Σ_hand net_result_i / big_blind_i) / count(hands) × 100`
(i.e. `Σ net_result` divided by the per-hand big-blind normalization, per 100 hands).
**Opportunity:** none — derived rate over the same filtered hands as Net result.
**Edge cases:** multi-stake filters weight each hand by its own blind level (see
Conventions); the stake-grouped report is where BB/100 is meaningful per stake.
A hand's `big_blind == 0` is impossible in cash NLHE; if a corrupt row ever had one,
the row is excluded by the storage layer and the anomaly is counted in quarantine,
never silently (domain helper `bb100_factor()` guards this).

### €/100
**Definition:** `Σ net_result / count(hands) × 100` in minor units, displayed as
`€X.YZ` per 100 hands.
**Opportunity:** none — same filtered population as BB/100.
**Edge cases:** v1 assumes one EUR account (`Currency::Eur` only; ADR-0003). Display
is always formatted with the `€` symbol per the hand history's currency. If a second
currency ever appeared (out of v1 scope), this statistic is undefined and must be
suppressed per currency, never summed across currencies.

### Big blinds won (cumulative)
**Definition:** `Σ_hand net_result_i / big_blind_i` — the un-normalized (per-100 not
applied) big-blind total that the cumulative graph plots.
**Opportunity:** same population as BB/100.
**Edge cases:** identical to BB/100 including the per-hand blind normalization. On
the graph, the x-axis is UTC hand time; the all-in adjusted line (below) is the
second series on the same scale.

### All-in adjusted (EV) result
**Definition:** `Σ adjusted_result` over filtered rows. Per hand, if the hero took
part in at least one all-in-contested side pot with known cards:
`adjusted_result = net_result + Σ_p (E_p(hero) × amount_p − actualAward_p(hero))`
summed over all-in-contested pots `p` where the hero is eligible. Otherwise
`adjusted_result = net_result` and `luck_delta = 0`.
**Opportunity:** there is an EV opportunity for a side pot `p` iff (a) the hero is
eligible for `p`, (b) at the moment the last betting action between the participants
of `p` concluded, **every participant eligible for `p` was all-in** (no more betting
was possible in that pot), (c) all of those participants' hole cards are known
(revealed at/showed for showdown), and (d) the hero did not fold. Equity is computed
**once, at that all-in moment** — never per street, never updated after (spec story
33, and `src/ev`).
**Edge cases:**
- *All-in without further cards* (all-in on the river, or all-in where the runout is
  already complete): equity is exactly 0 or 1; `adjusted = actual`, delta 0. The hand
  is still an EV opportunity, it just contributes zero luck.
- *Unknown cards*: if any participant eligible for the contested pot did not show
  cards (mucked without showdown, disconnected muck), no adjustment is possible for
  that hand; actual result stands, delta 0, and the hand is excluded from the EV
  population (it is not a zero-luck sample of a known-equity race).
- *Side pots*: each side pot is adjusted independently with its own eligibility list
  and its own amount. A player all-in for less creates a side pot the short stack is
  not eligible for; equities in a side pot are computed **only among that pot's
  eligible players with known cards** — never mixed across pots (story 34).
- *Hero not all-in*: if the hero still had chips behind at the last action (the hero
  is merely covering an opponent's all-in), the hero had no EV moment; the hand's
  own portion is not adjusted (`adjusted = actual`).
- *Folded or uncalled all-in*: hero folds, or the hero's all-in was never called
  (excess returned): no contested pot, no adjustment.
- *Equity and rounding*: equity is computed by exhaustive enumeration of all
  remaining runout boards when the card space is small enough, otherwise by a
  deterministic Monte Carlo with a fixed documented seed (see `src/ev/mod.rs`).
  **Ties split equally** among the tied eligible players for that side pot. The
  expected award for each pot is computed in exact rational arithmetic from
  enumeration counts and then **rounded half-to-even ("banker's rounding") to whole
  minor units** before storage. Equity percentages shown in the UI (if any) are the
  enumeration ratios rounded to 0.1%. Reconciliation tolerance against PT4 must
  absorb at most ±0.005 € of rounding per pot (Ambiguity log A9).
- `adjusted_result` is stored per hand at import time; nothing street-by-street is
  ever recomputed at query time (spec, Data model).

### Luck delta ("actual minus adjusted")
**Definition:** `Σ net_result − Σ adjusted_result` over filtered rows; per hand
`luck_delta = net_result − adjusted_result` (stored field, zero when there was no
adjustment).
**Opportunity:** none — derived. Over any filter set, `Σ luck_delta = Σ over all
all-in-contested pots of (actual − expected)` and is therefore non-zero exactly on
the hands that had an EV opportunity.
**Edge cases:** sign convention: **positive = hero ran above the all-in EV line
(running good)**; negative = below. Cumulative graphs plot net result and the
adjusted line; the "luck" figure is the gap between them (story 35). Because a hand
with no EV opportunity carries delta 0, the luck delta over a filter is unaffected
by hands the hero folded or hands without a race.

### Rake attributed (weighted contribution)
**Definition:** `Σ rake_attributed` over filtered rows. Per hand:
`rake_attributed(hero) = round-half-even(hand.rake × hero.contributed / Σ_seats
seat.contributed)` (the sum over **all seats** that put chips in).
**Opportunity:** computed at import for every dealt-in hand from seat contribution
totals (`parsed_seat.contributed`); attribution is stored, so every report that
shows rake reads the same stored number and reports can never disagree (spec,
Data model).
**Edge cases:**
- `contributed` counts every chip the seat put in across all streets
  (blinds included) **excluding returned uncalled bets**.
- `Σ seat.contributed` is the total contested pot before rake removal; rake is never
  larger than that total (property test).
- If `Σ seat.contributed == 0` the hand has no rake; attribution stays 0.
- If the hand's summary shows a **separate jackpot/BBJ drop** (not folded into
  rake), the drop is parsed into `jackpot_drop`, attributed by the same weighted-
  contribution formula into `jackpot_attributed`, and displayed separately. Rake
  statistics above **never include the jackpot drop** (spec, Further notes; field
  contract `ParsedHand.jackpot_drop` / `HeroHandAnalysis.jackpot_attributed`).
- A seat that contributed nothing is attributed 0 rake.
- Rounding of the per-seat attribution uses half-to-even at the last step; the
  sum of per-seat attributions must equal the hand rake exactly (property test) —
  any residual cent from rounding is assigned to the largest contributor.

### Rake in big blinds
**Definition:** `Σ_hand rake_attributed_i / big_blind_i` — rake expressed as a
cumulative number of big blinds.
**Opportunity:** same rows as rake attributed.
**Edge cases:** per-hand blind normalization, as with BB/100. This statistic exists
so rake cost is comparable across stakes (story 37).

### Showdown winnings
**Definition:** `Σ sd_winnings` over filtered rows where `went_to_showdown == true`;
by construction `sd_winnings = net_result` on showdown hands and 0 otherwise.
**Opportunity:** a hand went to showdown — showdown cards were compared and at least
two live hands existed at the showdown. An all-in runout that ends with cards shown
**is** a showdown hand for this split.
**Edge cases:** a hand the hero folded preflop contributes its (negative) net result
to **non-showdown winnings**, never to showdown winnings.

### Non-showdown winnings
**Definition:** `Σ non_sd_winnings` over filtered rows where
`went_to_showdown == false`; by construction `non_sd_winnings = net_result` on
non-showdown hands and 0 otherwise.
**Opportunity:** the complement of showdown winnings; the two always sum to
`Σ net_result` over the same filter (property the UI asserts, story 55).
**Edge cases:** hands won uncontested (last aggressor wins, no cards compared) are
non-showdown hands even if they reached the river.

---

## Preflop statistics

### VPIP (voluntarily put chips in pot)
**Definition:** `Σ vpip / hands played` over the same filtered rows (the domain
schema has no separate `vpip_opportunity` flag — the denominator is the hand count
itself).
`vpip` is set when the hero **voluntarily** put chips in preflop: any call, bet, or
raise (including SB completion, and including raises that are 3-bets, 4-bets, or
all-ins).
**Opportunity:** dealt in. Every hand the hero was dealt into is a VPIP opportunity
(PT4 denominator = hands played).
**Edge cases:**
- *Big blind free check*: the BB checking preflop (option) is **not** a VPIP and
  **not** a limp — no chips moved voluntarily. It is still an opportunity and the
  hand still counts in the denominator.
- Dead blind posts and blind posts are not voluntary action.
- A hand that ends before the hero acts (folds to the BB, BB wins uncontested) is an
  opportunity with no numerator.
- All-in preflop for the opening raise counts (it is voluntary aggression).
- Folding, or check-folding the SB (SB only posts and folds), is not VPIP.

### PFR (preflop raise)
**Definition:** `count(pfr) / hands played` over the same rows.
`pfr` is set when the hero made any preflop raise, including 3-bets, 4-bets,
isolation raises, and all-in raises (uncalled raises included).
**Opportunity:** dealt in — the same denominator as VPIP (PT4 convention: PFR is a
subset of VPIP; a player cannot raise without putting money in).
**Edge cases:** blind posts are not raises; SB completing is a call, not a raise.
A raise that was returned uncalled still counts.

### Limp % (VPIP−PFR gap)
**Definition:** displayed as `VPIP% − PFR%` computed from the aggregate counts,
i.e. `(Σ vpip − Σ pfr) / hands played`. Both parts use the identical denominator so
the gap is never a subtraction of differently-populated numbers.
**Opportunity:** dealt in (same as VPIP/PFR).
**Edge cases:**
- The underlying per-hand flag `limp` is set when the hero voluntarily called the
  big blind in an unraised pot (SB completing included; SB/BB completing over an
  all-in shove below the blind is still a call of their own committed amount).
- *BB check preflop must NOT count as a limp or a VPIP* — checked option puts no
  voluntary money in (spec-mandated edge case; see VPIP above).
- Cold calls (calling a **raise**) are not limps.
- In a limped pot, any preflop raise still sets `pfr`, moving the hand out of the
  gap (raise no longer counts as passive).

### Raise First In (RFI / open-raise rate)
**Definition:** `count(rfi) / count(rfi_opportunity)`.
`rfi` is set when the hero made the **first voluntary contribution** to the pot
preflop by raising (or raising all-in). Only one seat per hand can have RFI.
**Opportunity:** `rfi_opportunity` is set when, at the hero's preflop decision
point, **no other seat had put voluntary chips in the pot** (only folds and blind
posts so far) and the hero had a decision point. A fold under these conditions is a
declined RFI opportunity; so is the BB option when everyone folded to the BB (the
BB could have raised; checking is not RFI).
**Edge cases:**
- If anyone limped or called before the hero, the hero has **no** RFI opportunity
  (a raise over a limper is an isolation raise, not RFI).
- If anyone raised before the hero, the hero has no RFI opportunity (a raise over a
  raise is a 3-bet).
- Uncalled opens count (an uncalled raise is still RFI).
- The BB winning a walk (folds to the BB, BB checks) is an RFI opportunity with no
  numerator — resolved this way because the BB had a genuine raise option
  (Ambiguity log A1).

### 3-Bet
**Definition:** `count(three_bet) / count(three_bet_opportunity)`.
`three_bet` is set when the hero raised over an open raise (the first raise over
the opening raise), including all-in jams and uncalled 3-bets.
**Opportunity:** `three_bet_opportunity` is set when the hero has a preflop decision
point facing **exactly one raise** (an uncalled open) that is not the hero's own,
regardless of position and regardless of whether a limper or blinds sit in front.
Includes blinds facing a steal/open and limpers facing an isolation raise.
**Edge cases:**
- Facing a *re-raise* (a 3-bet) is a 4-bet opportunity, not a 3-bet opportunity.
- Calls (open calls / limped-flat) and folds count in the fraction's denominator.
- Blind raises over another position's open are plain 3-bets; a 3-bet never
  requires that the 3-bettor be out of position or in position.

### Fold to 3-Bet
**Definition:** `count(folded_to_three_bet) / count(faced_three_bet)`.
`faced_three_bet` is set when the preflop aggressor (the hero's own open or any
raise the hero made that stood unraised) had a raise come over it, i.e. the hero
made an opening raise that someone re-raised, or the hero 3-bet and someone 4-bet
over it. `folded_to_three_bet` is set when the hero's **final response** to that
facing was to fold (no further voluntary money from the hero), whether the fold was
immediate or after an intermediate raise by another seat or a re-raise war the hero
exited.
**Opportunity:** the hero must have made the raise that got re-raised (you cannot be
3-bet without having raised). The blind **defenses**: a blind that simply *calls* an
open granted a call is a "cold call," not a fold-to-3bet (no re-raise happened).
**Edge cases:**
- If the hero called the 3-bet (or 4-bet all-in), `faced_three_bet` is counted but
  `folded_to_three_bet` is not.
- If the hero open-shoved all-in and was re-raised, the hero had no further
  decision; the re-raise window only exists if the hero still had chips.
- A "raise, re-raise, fold" line counts as fold-to-3bet (final decision governs).

### 4-Bet
**Definition:** `count(four_bet) / count(four_bet_opportunity)`.
`four_bet` is set when the hero raised over an existing 3-bet (the second raise over
the opening raise), including jams and uncalled 4-bets.
**Opportunity:** `four_bet_opportunity` is set when the hero has a preflop decision
point facing **exactly one raise over the existing raise** (facing a 3-bet, i.e., a
raise that is itself the second raise) — any seat, including blinds.
**Edge cases:** facing the *third* raise over the raise (a 4-bet jam) is a 5-bet
window, not a 4-bet opportunity; v1 folds 5-bet-or-higher into "faced/folded to 3/4
bet" only as far as store fields allow (PT4 counts them as 4-bet+; see Ambiguity
log A4). Calls and folds count in the denominator.

### Fold to 4-Bet
**Definition:** `count(folded_to_four_bet) / count(faced_four_bet)`.
`faced_four_bet` is set when the hero's own raise had a **second** raise over it
(a true 4-bet), i.e. the hero was 4-bet. `folded_to_four_bet` is set when the hero's
final response to that 4-bet was to fold.
**Opportunity:** symmetric to fold-to-3bet; the hero must have been the raiser whose
raise got 4-bet.
**Edge cases:** identical structure to fold-to-3-bet's edge cases, including
"final decision governs" and call/no-fold rules.

### Cold call
**Definition:** `count(cold_call) / count(cold_call_opportunity)`.
`cold_call` is set when the hero **called a raise** (not a 3-bet or 4-bet window)
without having raised, from any seat — including blinds calling an open (blind
defense by call).
**Opportunity:** `cold_call_opportunity` is set when the hero has a preflop decision
point facing exactly one uncalled raise that is not the hero's own — the same
situations that grant a 3-bet opportunity; calling instead of folding/raising counts.
**Edge cases:**
- A blind's call of an open counts as a cold call (documented PT4 behavior; see
  Ambiguity log A7).
- The BB's *check* (option) is not a cold call, not a VPIP, and not a limp.
- Folding a faced raise takes the opportunity away but adds no numerator.

### Steal attempt
**Definition:** `count(steal_attempt) / count(steal_opportunity)`.
`steal_attempt` is set when the hero opened (raised first in per RFI above) from a
steal seat. Steal positions are the last three seats before the blinds in acting
order: **CO, BTN, SB** (both 6-max and 9-max; see Ambiguity log A8 for whether HJ
belongs here in full ring).
**Opportunity:** `steal_opportunity` is set when the hero had an **RFI opportunity
from a steal seat** all action so far folds (no limpers, no raise).
**Edge cases:** action must be first-in; an isolation raise over a limper from
BTN is not a steal attempt (it is RFI, not a steal). SB opens count. BB has no
steal opportunity (BB's first action is an option or a defense, never first-in).

### Fold to Steal / Blind defense
**Definition:** `count(folded_to_steal) / count(faced_steal)`.
`faced_steal` is set when the hero is in a **blind** (SB or BB) and an open from a
steal seat (CO/BTN/SB per above) came over the hero's blind. `folded_to_steal` is
set when the hero's response to that steal attempt was a fold.
**Opportunity:** blind seats facing a steal-position open. Blind seats facing opens
from UTG–HJ are **not** fold-to-steal opportunities (they are generic raises).
**Edge cases:**
- Calling or 3-betting the steal counts in the denominator, not the numerator.
- Fold-to-steal is **blind-only** (blind defense); a non-blind spot folding to an
  open from a steal seat is a generic fold, not this stat (Ambiguity log A5).
- The hero must still have chips to have faced an acting opportunity (an all-in
 blind has no defense decision).

---

## Postflop statistics

Attribution rule used by several stats: **only the preflop aggressor can c-bet**.
If the preflop aggressor is not present in the portion of the pot being bet (they
are all-in and therefore absent from a side pot, or they have no decision left),
**the c-bet opportunity does not exist** for anyone else. Fold-to-c-bet
opportunities are granted only against bets that meet this c-bet rule.

### C-Bet Flop
**Definition:** `count(cbet[0]) / count(cbet_opportunity[0])`.
`cbet[0]` is set when the hero was the preflop aggressor and made the **first
bet** on the flop (or the raise/all-in that is the flop's first aggression).
**Opportunity:** `cbet_opportunity[0]` is set when the hero is the preflop
aggressor, the flop was dealt, the hero has an acting opportunity on the flop (not
all-in), and at least one live opponent is able to act.
**Edge cases:**
- *Aggressor not present*: if the hero went all-in preflop (no flop decision,
  no flop c-bet) — and no other seat gains a c-bet opportunity either (a bet by
  another seat when the aggressor is absent is not a c-bet and grants no
  `faced_cbet` either).
- *Limped pots*: no aggressor exists; no c-bet opportunity exists (checks and bets
  in such pots have no c-bet name; they may count as probes on later streets if
  raised — see Probe).
- *Aggressor all-in for less*: the aggressor's all-in bet into the **main pot
  portion** the aggressor is eligible for is still the flop c-bet; a different
  seat's bet into a **side pot the aggressor is not eligible for** is not a c-bet
  and grants no fold-to-cbet opportunity.

### C-Bet Turn
**Definition:** `count(cbet[1]) / count(cbet_opportunity[1])`.
`cbet[1]` is set when the hero was the preflop aggressor **and c-bet the flop**,
and made the **first bet** on the turn.
**Opportunity:** `cbet_opportunity[1]` is set when the hero c-bet the flop, has an
acting opportunity on the turn, and no other bet had been made on the turn before
the hero's first turn decision. Checking the turn back consumes the opportunity
(opportunity granted, no numerator).
**Edge cases:** same "aggressor absent" and "limped pot" rules as C-Bet Flop. If
hero c-bets the flop but someone else bets the turn before hero's first turn
decision, hero has no turn c-bet opportunity (a bet already stood).

### C-Bet River
**Definition:** `count(cbet[2]) / count(cbet_opportunity[2])`.
`cbet[2]` is set when the hero c-bet flop **and** c-bet turn as the preflop
aggressor, and made the **first bet** on the river.
**Opportunity:** `cbet_opportunity[2]` analogously — hero c-bet the turn, acting
opportunity on the river, no earlier river bet.
**Edge cases:** same presence/limped-pot rules. A river bet by the aggressor after
checked-back turn is a value/play bet with no dedicated stat (not a c-bet, not a
delayed c-bet under this convention).

### Fold to C-Bet (by street)
**Definition:** `count(folded_to_cbet[s]) / count(faced_cbet[s])` per street
`s ∈ {flop(0), turn(1), river(2)}`.
`faced_cbet[s]` is set when the hero was facing a **bet by the preflop
aggressor** that was the first bet on street `s` (i.e. the aggressor's c-bet) with
the hero having an acting opportunity. `folded_to_cbet[s]` is set when the hero's
final response on that street was to fold.
**Opportunity:** hero must be able to act (not all-in) and at least one aggressor
bet must stand unraised when the hero's decision arrived.
**Edge cases:**
- Bets by non-aggressors (the flop caller betting the turn, the aggressor absent,
  side-pot-only bets) are **not** c-bets and grant no fold-to-cbet opportunity.
- Hero raising then folding on the same street still counts as fold-to-cbet (final
  decision governs, mirroring fold-to-3bet).
- Folding on a later street to a later bet is fold-to-the-later-street's bet, not a
  fold to this c-bet.
- If the aggressor's c-bet was itself an all-in and the hero has chips behind, the
  hero still has an acting opportunity and the fold counts.

### Check-Raise (by street)
**Definition:** `count(check_raise[s]) / count(check_raise_opportunity[s])`
per street `s ∈ {flop, turn, river}`. The numerator `check_raise[s]` is set when
the hero **checked** on street `s` and later, still on street `s`, **raised**
over a bet that arrived after the hero's check.
**Opportunity:** `count(check_raise_opportunity[s])` is stored per street
(`HeroHandAnalysis.check_raise_opportunity`) and is set exactly once when the hero
checked street `s` with an acting opportunity, at least one opponent could still
act (raise), and a bet the hero could have raised actually arrived on that street.
The check must be a live check — not a check into an all-in bet that ended
betting, and not a spelling out the street with nothing left to face.
**Edge cases:** check-then-call is an opportunity with no numerator; a
check-raise jam counts; a raise by the hero *before* any check on the street is a
bet/raise, not a check-raise.

### Probe Bet
**Definition:** `count(probe[s]) / count(probe_opportunity[s])` for `s = turn(1)`,
`river(2)` (the flop index is structurally always opportunity-free in our schema —
a flop's first bet by a non-aggressor is a plain bet, with no probe name).
`probe[s]` is set when: the pot was **raised preflop** (an aggressor exists), the
hero is **not** the preflop aggressor, the immediately preceding street(s) were
**checked through** (zero betting on those streets in this pot), the hero has an
acting opportunity on street `s`, and the hero made the **first bet** on `s`.
**Opportunity:** exactly the conditions above minus the bet itself — checked
through before `s`, hero's turn to act with the pot unbet, hero can act, at least
one live opponent can act.
**Edge cases:**
- *Heads-up / multiway parity*: the definition is indifferent to table size — one
  live, chip-capable opponent is enough; in a three-way pot a probe counts the same
  as heads-up.
- *Aggressor folded postflop*: the probe still counts on later streets (the
  aggressor's preflop raise defined the raised pot), unless the aggressor is
  all-in and absent from the side pot being bet — then the "pot is raised" anchor
  is gone for that side pot and no probe opportunity is granted.
- *Limped pots*: no preflop aggressor → no probe opportunity, on any street.
- If the aggressor bets instead (delayed c-bet) the same action is *theirs*, so the
  two stats are mutually exclusive for the same action.

### Delayed C-Bet
**Definition:** `count(delayed_cbet[s]) / count(delayed_cbet_opportunity[s])` for
`s = turn(1)`, `river(2)` (flop index unused, like probe).
`delayed_cbet[s]` is set when: the hero was the **preflop aggressor**, did **not**
c-bet the flop (flop checked through with zero flop betting), the hero has an
acting opportunity on street `s`, no earlier bet on street `s`, and the hero made
the **first bet** on `s`.
**Opportunity:** same conditions minus the bet itself.
**Edge cases:**
- Mutually exclusive with C-Bet Turn/River by construction: a hand where the hero
  c-bet the flop then bet turn is a turn c-bet, not a delayed c-bet; if the flop
  was checked through, a turn bet by the hero is a delayed c-bet.
- If the aggressor bet river after check-check flop and turn, that is a delayed
  river c-bet under this convention.
- Limped pots (no aggressor) grant no delayed c-bet opportunity to anyone.

### WWSF (Won When Saw Flop)
**Definition:** `count(won_when_saw_flop) / count(wwsf_opportunity)`.
`wwsf_opportunity` is set when the hero **saw the flop** — the hero was dealt in,
was still live at the flop, and the flop was actually dealt (a hand that ended
preflop such as a walk, or a hero fold preflop, does not qualify, even though the
flop cards were dealt for others).
**Edge cases:**
- An all-in preflop that runs out to the flop **counts** the hero as having seen
  the flop (the hero's cards were live past the flop deal).
- The BB's check (option) carrying through to the flop **counts** the hero as
  having seen the flop, even though no chips moved preflop.
- *Won* is the pot-based rule from Conventions (`won_amount > contributed`),
  which excludes returned uncalled bets.
- Split pots where hero's share exceeds hero's contribution count as wins; a
  split where hero exactly broke even does not.

### WTSD (Went To Showdown)
**Definition:** `count(went_to_showdown) / count(wwsf_opportunity)`.
`went_to_showdown` is set when the hand reached the showdown and the hero was still
live at it (never folded during the hand). The denominator is the same
`wwsf_opportunity` count (PT4: WTSD is conditional on seeing the flop).
**Edge cases:**
- All-in runouts that end in cards shown count as showdowns for this stat.
- Hero folded preflop/postflop then the flop/turn/river and showdown happened
  without the hero: not a WTSD numerator, but the hand's WWSF denominator also
  does not include the hero (both stats are about hands the hero stayed in).
- A hero winning uncontested on the river (no showdown) is not a WTSD numerator;
  the hand is a non-showdown hand for the winnings split too.

### W$SD (Won At Showdown)
**Definition:** `count(won_at_showdown) / count(went_to_showdown)`.
`won_at_showdown` is set when `went_to_showdown` is true **and** the hero won a pot
by the pot-based rule (`won_amount > contributed`).
**Opportunity:** showdown hands the hero stayed in (see above).
**Edge cases:** split pots: a share exceeding the hero's contribution is a win; an
exact break-even split is not (pot-based definition, as WWSF). Side pots: winning
any pot the hero was eligible for (main or side) counts as a win.

### Aggression Factor (AF)
**Definition:** `Σ aggressive_actions[s] / Σ passive_actions[s]` per street `s`, and
overall as the flop+turn+river sums:
**Overall AF** = `(Σ_{s∈{flop,turn,river}} aggressive_actions[s]) /
(Σ_{s∈{flop,turn,river}} passive_actions[s])`.
In `HeroHandAnalysis`, `aggressive_actions` counts bets + raises + all-in bets/raises
(+ all-in check-raises, which are raises) on that street; `passive_actions` counts
**calls only** (including calls that follow a check).
**Opportunity:** each action is its own unit; the quotient is over all actions taken
on those streets in the filtered hands (a per-street count style, not a per-hand
percentage).
**Edge cases:**
- **Preflop index 0 exists** in the schema and is excluded from the *overall* AF
  denominator/numerator, matching the HUD convention that preflop aggression is
  measured by VPIP/PFR/3bet/4bet — but per-street AF for preflop is still derivable
  (Ambiguity log A10).
- If `passive_actions == 0` and `aggressive_actions > 0`, the factor is shown as
  `∞` (never as an error).
- Checks are neither numerator nor denominator in AF.
- Reported **both** as factor and frequency because they diverge under small
  samples; the UI shows both (spec, Reports and queries).

### Aggression Frequency (Agg%)
**Definition:** `Σ aggressive_actions / (Σ aggressive_actions + Σ passive_actions)`
per street, and overall over flop+turn+river (same sums as overall AF).
Equivalent algebra: `Agg% = AF / (AF + 1)` when both share the same counts — a
rescaling, not independent information; the UI shows both because players read
them at different scales.
**Opportunity:** same action counts as AF.
**Edge cases:**
- Same preflop-exclusion rule as overall AF (A10).
- With the domain's `passive_actions` counting calls only, the denominator does
  **not** include checks — a deliberate deviation from PT4's variant that counts
  checks as passive actions in Agg%'s denominator; adding a `checked_actions`
  counter to the domain schema would be required to match PT4 exactly (Ambiguity
  log A11).

---

## Position, table size, and session (definitions, not tables)

### Position
Positions are assigned from the button. **SB, BB, BTN are fixed**; the seats
between BB and BTN are labelled, in preflop acting order, by
`Position::middle_sequence(n)` where `n` is the dealt-in table size. The enum
(`Position::Sb, Bb, Utg, Utg1, Utg2, Mp, Hj, Co, Btn`, 9 variants) maps exactly
one variant per seat on every supported table size:

| Table size | Seats, acting order after the button |
|---|---|
| 6 (supported) | SB, BB, UTG, MP, CO, BTN |
| 7 | SB, BB, UTG, UTG1, MP, CO, BTN |
| 8 | SB, BB, UTG, UTG1, UTG2, MP, CO, BTN |
| 9 (supported) | SB, BB, UTG, UTG1, UTG2, MP, HJ, CO, BTN |

So in 6-max the middle seat (2nd to act) is `Position::Mp` (PT4-style naming;
some tools call it HJ, but PT4's 6-max uses MP); in 9-max `Hj` is the hijack
directly left of CO. `Utg1` = UTG+1, `Utg2` = UTG+2. Heads-up is out of v1 scope
(and at 2-max the button posts the small blind and is labelled BTN, the other
seat BB — no SB row exists at 2 seats).

### Table size
`ParsedHand.table_size` / `HeroHandAnalysis.table_size` — 6 or 9 for the supported
formats. Steal-seat membership and position mapping depend on it, not on the table
name or the number of seats actually dealt.

### Session
Sessions are derived at query time from the hero's hands sorted by
`timestamp_utc`, splitting whenever the gap between consecutive hands is
**≥ the configured idle gap (default 30 minutes = 1800 s)**. Multi-tabling merges
naturally (the gap is time-based, not table-based). Fields
(`Session { started_at, ended_at, hands, net_result, currency }`): `started_at` =
first hand UTC, `ended_at` = last hand UTC, `hands` = dealt-in count, `net_result`
= sum of hand nets. **Duration is `ended_at − started_at` and is labelled
approximate in the UI** (stories 66–71). All comparisons and the idle gap are
computed in **UTC**; original local timestamp strings are preserved on the hand for
display only. Daylight saving and travel therefore never shift session grouping.
Session ids (decimal integers) are the sequential index of the session within the
current filter's hand ordering — **stable for a given filter set**, not globally
absolute; `FilterSpec::sessions` refers to them (Ambiguity log A3). v1 assumes a
single EUR account; a session is never split by currency because all rows share
one currency.

---

## Ambiguity log (spec or PT4-unspecified; resolutions recorded)

- **A1 — walk / fold-to-BB opportunity.** The spec does not say whether a hand
  where everyone folded to the BB counts as a preflop opportunity for the BB.
  Resolved: yes, it counts as an RFI/VPIP/PFR opportunity (the BB had a genuine
  raise option) with no numerator. PT4 appears to do the same; confirm during
  reconciliation.
- **A2 — full-ring position granularity.** ~~Resolved as a collapse~~ **Resolved
  in code:** `Position` now has 9 variants (`Utg, Utg1, Utg2, Mp, Hj, Co` between
  the blinds and the button) and `Position::middle_sequence(n)` assigns exactly
  one variant per seat per table size — 6-max `UTG, MP, CO`, 7-max `…UTG1, MP, CO`,
  8-max `…UTG2, MP, CO`, 9-max `…UTG2, MP, HJ, CO` (see the Position table). Every
  9-max seat is now distinct, matching PT4's granularity; nothing further to
  confirm beyond normal reconciliation.
- **A3 — session identity.** The spec does not define a global session id.
  Resolved: session ids are ordinal within a filter, matching `Session.id` and
  `derive_sessions` in `src/stats`. Rebuilding ids across different filters is
  undefined — filters that include a session id must use that same filter's
  ordering.
- **A4 — 5-bet and higher.** The domain schema stores one `four_bet` flag and no
  5-bet/6-bet arrays. Resolved: v1 counts 5-bets and higher as `four_bet` /
  `folded_to_four_bet` events (PT4 rolls them into the same "4-bet+" bucket). An
  exact 5-bet breakdown would need a schema change; flagged for follow-up.
- **A5 — fold-to-steal audience.** PT4's "fold to steal" is blind-only in common
  HUD implementations. Resolved: blind defense only, per the ticket's wording.
  If reconciliation shows PT4 counting MP folds to a BTN open, this needs a
  schema tweak (`faced_steal` gating widened), not a definition change.
- **A6 — "won a pot" vs "net positive".** WWSF/W$SD use `won_amount >
  contributed`. PT4's "won" is net-positive in money. The two differ only when a
  win is exactly offset by rake (net = 0 exactly). Resolved in favor of the
  pot-based rule; the razor-edge is accepted as a known divergence.
- **A7 — cold-call for blinds.** PT4's "cold call" description is ambiguous about
  blind calls of an open. Resolved: blinds' calls count as cold calls (common HUD
  behavior). If PT4's numbers exclude them, tighten `cold_call_opportunity` by
  seat during reconciliation.
- **A8 — steal seats in full ring.** PT4 sometimes includes HJ in full-ring steal
  opportunities. Resolved: CO/BTN/SB in both formats. May need widening after
  reconciliation.
- **A9 — EV equity/rounding.** PT4's exact Monte Carlo count and rounding for
  exact-enumeration multiway side pots are not publicly documented. Resolved
  (our convention, documented for reconciliation tolerance): enumeration where
  exact, deterministic seeded Monte Carlo otherwise; ties split equally;
  expected awards rounded half-to-even to whole minor units per side pot; the
  same seed and sample size are fixed constants in `src/ev`. The ±0.005 €/pot
  rounding bound is part of the reconciliation tolerance.
- **A10 — overall AF including preflop.** PT4's "overall AF" description does not
  explicitly say whether preflop folds into it. Resolved: overall AF and overall
  Agg% exclude preflop (indices 1..3 of the aggression arrays); per-street AF/Agg%
  for preflop remain available from index 0 for the positional report.
- **A11 — Agg% denominator including checks.** PT4 counts checks among passive
  actions in Agg%'s denominator; the `HeroHandAnalysis` schema has no
  `checked_actions` counters. Resolved: Agg% denominator = aggressive + calls
  only, which makes Agg% = AF/(AF+1) exactly. Matching PT4 exactly requires a
  domain schema addition; flagged for follow-up, not silently changed.
- **A12 — `jackpot_attributed` display.** The spec says to verify the jackpot
  drop against real files. Resolved (defensive): parse it separately
  (`ParsedHand.jackpot_drop`), attribute it alongside rake by weighted
  contribution (`jackpot_attributed`), and never include it in any rake or net-
  result statistic. If no real-world hand ever carries a drop line, the field
  stays zero and the stat is simply not shown.
- **A13 — check-raise opportunity was not storable.** **Resolved in code:**
  `HeroHandAnalysis` now carries `check_raise_opportunity: [bool; 3]` (and the
  `hero_hands` schema the matching column), so check-raise is a proper rate per
  street — `check_raise[s] / check_raise_opportunity[s]` — aggregated at query
  time like every other opportunity-gated stat. The opportunity is "hero checked
  street s with an acting opportunity and a bet they could raise arrived on that
  street" (see Check-Raise).
