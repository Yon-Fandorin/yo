---
schema: methexis.knowledge/v1alpha1
id: methexis.workflow.self-hosting-boundary
kind: rule
owner: methexis
sources:
  - id: methexis.workflow-model.self-hosting-boundary
    revision: sha256:5b9f7b56472fc02d0c82591d5c29cb11d2e030989e890740d94c97fe68a1f1ba
---
# Workflow self-hosting boundary

## Statement

`CONTRIBUTING.md` remains the sole workflow authority during the Pilot. A future
explicit migration MAY make approved workflow KnowledgeUnits canonical and
commit `CONTRIBUTING.md` as their human-readable `DocumentView` Projection.

That migration requires:

- complete rule coverage and semantic-equivalence review;
- explicit human approval of the generated document;
- a generation-drift check in the repository validation path;
- a pinned last-known-good tool and documented recovery procedure;
- one atomic owner transition that removes dual authority.

After migration, contributors change the owning KnowledgeUnits and regenerate
the committed Projection rather than editing it independently. The generated
file remains readable without running Methexis.
