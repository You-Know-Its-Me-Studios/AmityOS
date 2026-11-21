# Phase 0 acceptance: plan

Date: 2026-11-20. Reviewed: the repository as it stood before the restructure.

## Done

- **Section 2 verified against the code:** line counts, components, dev host and storage
  rule. Corrections are in [migration plan](../migration/plan.md) section 1: window
  protocol v4, C ports are adapters with upstream trees in `.deps/`, plus unlisted
  modules, user data in `artifacts/` and uncommitted work.
- **Keep/port/retire table** adjusted to the real modules: [migration plan](../migration/plan.md)
  section 2.
- **Decision records** [0001](../decisions/0001-linux-base.md) to
  [0007](../decisions/0007-apps-and-packaging.md), each with licences.
- **Screenshots:** the folder `C:\Users\Josh\Downloads\AmityOS-Screens` has 38
  screenshots. Three were opened (01, 04, 07) to check the toolkit decision; the full
  review and inventory are Phase 4 work.

## Not done or not verified

- The current Ubuntu LTS release (26.04) is from planning-time knowledge. Phase 2 pins
  and confirms it.
- No code was changed, deleted or built.

## Owner decisions

The conflicts raised in this phase and the owner's decisions are recorded in
[migration plan](../migration/plan.md) section 4.
