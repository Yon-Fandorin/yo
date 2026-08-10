---
schema: methexis.knowledge/v1alpha1
id: methexis.interface.operation-chain
kind: rule
owner: methexis
sources:
  - id: methexis.interface-model.operation-chain
    revision: sha256:c61c482eea63cd4e951ca0f49a8aa15e7a4fbfeef18126ddca0bdeb2b855c04f
---
# Methexis operation chain and authority boundaries

## Statement

`semantic-first-ko-on-demand/v1` is exposed only for the complete minimal flow: `author-revision` writes Source and canonical English Knowledge Drafts; after repository-owned semantic review clears, `project-review` generates Korean only on explicit human request; `build-review` presents the exact English and Korean pair; and `prepare-approval` plus `approve` retain proposal and exact-human-authorization boundaries.

The capability selects the current operation path and creates no durable authority or artifact lineage. Without it, the legacy flow remains controlling and still requires exact-revision human approval. Existing legacy records remain valid for the exact revisions they bind without bulk migration.

Agent-review procedure, reviewer-session handling, and review evidence are owned exclusively by the repository workflow authority. Methexis consumes only the resulting workflow disposition and does not define a second provider-attestation or reviewer-routing policy.

After semantic review clears, `project-review` publishes Korean for the exact current revision on human request. Any semantic change restarts semantic review; a translation-only change repeats human review. Other prepare, Checkpoint, activation, validation, and ContextBuild boundaries are unchanged.
