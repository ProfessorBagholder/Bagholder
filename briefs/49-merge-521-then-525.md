# Brief 49: #525 at `4bec4024` is ready. Merge #521 first, then bring it into #525 and resolve three conflicts

## #525 at `4bec4024`: brief 47's four changes hold

**Checked here.** I built the branch and ran the entry, import and capacity tests and the engine's: 30 and 36 passed. Then I undid each change in turn and ran them again:

| Change undone | Test that fails |
|---|---|
| "None then, two now" takes the first | `a_trade_entered_on_a_listing_two_instruments_came_to_bear_is_refused` |
| `enter` ignores the refusal (`.unwrap_or(None)`) | the same test |
| the refusal always says "on" | that test, and `a_symbol_with_no_venue_is_refused_where_two_holdings_have_that_ticker` |
| an unknown exchange refuses the file | `a_files_row_naming_an_unknown_exchange_is_kept_with_its_problem_and_the_file_imported` |
| `backdated-file-row` left out of `waited_on` | `every_operation_a_person_waits_on_is_timed` |

`InstrumentInfo::name_on` is gone. Every cargo step in CI is `--locked`. CI is green on all eight jobs.

**Verdict:** #525 is ready.

## Merge order

The two PRs conflict in three files:
- `rust/crates/net/src/client.rs`;
- `rust/crates/net/src/net.rs`;
- `web/e2e/serve.mjs`.

#521 puts every request through one door and names each Wealthsimple call. #525 carries the offline refusal as a type through that same code.

1. **Merge #521.** It is smaller, and it is green.
2. **Merge master into #525** (a merge commit, no rebase), and resolve the three files so that both rules hold:
   - every request still leaves through `client::leave`, and its offline refusal is the typed one #525 made, never words;
   - `named_call` stays beside the `outbound` line, and `serve.mjs` keeps both #521's record of the calls answered and #525's changes.
3. **Re-run the suites on the merge,** in full: `cargo test --workspace --locked`, clippy and the full e2e.
4. **Run the planted cases that must still fail:**
   - #521's two: the order call pointed at the real address fails the host scan; a Wealthsimple call sent elsewhere fails the signed-in e2e;
   - #525's offline tests: `an_sec_document_read_offline_is_refused_by_its_kind`, `a_sedar_document_read_offline_through_the_live_readers_is_never_said`.

I will check the merge commit's three files, and those results, before #525 is merged.
