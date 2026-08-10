---
schema: methexis.knowledge/v1alpha1
id: methexis.interface.atomic-publication
kind: rule
owner: methexis
sources:
  - id: methexis.interface-model.atomic-publication
    revision: sha256:617c0bc99b1fe23728d7d8fb9506adabfea600b85bdf2a45f20b27acf48f3df0
---
# Atomic tracked mutation publication

## Statement

Every mutation publishes atomically and rejects symlinked output parents.
Publication resolves and retains directory handles before locking or writing;
a concurrent parent rename or symlink swap cannot redirect output outside that
opened repository directory.
Tracked mutations serialize concurrent writers per target. A different
Projection requires its exact prior content hash; a different approval requires
its exact prior RevisionId. Checkpoints are immutable; active-record replacement
requires the exact prior record hash. Failures leave the prior record unchanged
and expose no eligible partial output.
