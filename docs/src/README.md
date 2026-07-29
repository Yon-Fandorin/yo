# yo Developer Docs

This first documentation surface is for people and coding agents changing
`yo`. It is not yet the product guide for installing or using `yo`.

Use these docs to:

- locate the crate or module that owns a change;
- follow the runtime path across crate boundaries;
- find the relevant deterministic and environment-dependent checks; and
- reach the Methexis decision or contract that owns a design constraint.

## Choose the authority

| Question | Authority |
|---|---|
| What is `yo`, and what is currently public? | Repository [`README.md`](https://github.com/Yon-Fandorin/yo/blob/develop/README.md) |
| How do branches, Slices, reviews, and commits work? | [`CONTRIBUTING.md`](https://github.com/Yon-Fandorin/yo/blob/develop/CONTRIBUTING.md) |
| Where does code live, how does it run, and how is it validated? | These Developer Docs |
| Which accepted behavior or design constraint must remain true? | Methexis KnowledgeUnits |

Methexis KnowledgeUnits own accepted design decisions and behavioral
contracts. These Developer Docs own code navigation, working explanations, and
validation guidance. When a contract matters, this guide links to its
KnowledgeUnit instead of restating it as a second authority.

For a first change:

1. Use [Architecture](./architecture/overview.md) to learn the system shape.
2. Choose an owner from the [Code map](./architecture/code-map.md), or start
   from an observable outcome in [Find the change](./workflows/find-the-change.md).
3. Follow the [Runtime flow](./architecture/runtime-flow.md) if the change
   crosses boundaries.
4. Select focused and Slice-close evidence in
   [Validation](./validation/README.md).
