# Brief 68: #557 at `0a9c15f3`: not yet. Whose debts are read, and what sign a balance carries, rest on no recorded reply

## Verdict

**Not yet, two changes.** Brief 67's four changes are built as asked:
- owed only with every account in scope;
- whole-name equality after folding legal forms;
- the debit purposes split three ways from the bundle's 130 categories and tested as sets;
- the as-of date always on hover.

The live read returns a value or a named mismatch, never a quiet nothing. Only replay without a capture gives `None`.

**Dropping `FetchFundingMethods` is accepted.** A bill payment still matches by the account's last four digits from the activity row (`redactedExternalAccountNumber`), and otherwise by the company's whole name. The card-issuer flag added nothing the name rule doesn't give. A payee it can't settle is asked once, which is the designed fallback.

## Required changes

1. **Only the person's own debts.** The trimmed query reads `externalFinancialEntities` with neither `ownershipType` nor `visibility`. Wealthsimple's own document asks both (brief 23, `23/graphql/FetchIdentityNetWorthAccounts.graphql`), and the feature has households ("Create a Wealthsimple household"). A household member's debt shared into the person's view would then count as the person's.
   - Ask `ownershipType` and `visibility { household }`.
   - Read the enum's values from the bundle (the method of brief 23 §2), and keep only the entities the person owns. A value outside the known set is a named mismatch, never a guess.
   *Test:* a generated reply with a household member's entity and a joint one. Only the person's own count, and joint counts per the enum's meaning, said in SPEC.
2. **Shape and sign come from a real reply before this merges.** Nothing recorded shows what `currentBalance` holds for a debt: a positive amount owed, or a negative balance. If it is negative, "owed" goes negative and "net" goes up. That is a wrong figure on the Portfolio tab, and no strict read catches it, because the shape would be fine. The plan's own criterion is "reply shapes recorded from your session and anonymised", and it isn't met.
   - One read-only capture of `FetchIdentityNetWorthAccounts` through the owner's own session, anonymised with `ws-anonymise`, decides the sign rule. The rule is written from it and tested against it.
   - It also gives the plan's settle count: counts only.
   - Under `CLAUDE.md`'s gate, a criterion the session can't run is named at the PR's top as the one capture the owner supplies. #557 waits for it.

Everything else is cleared. One push after the capture.
