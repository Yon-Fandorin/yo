---
schema: methexis.knowledge/v1alpha1
id: methexis.product.identity-terms
kind: rule
owner: methexis
sources:
  - id: methexis.product-model.identity-terms
    revision: sha256:3c8c6a394691e985b0fe5fd612184b06b7050beb6cafb11dec810b29e622c7aa
---
# Methexis product identity and SOT terms

## Statement

Methexis is the product that maintains agreement between approved canonical
knowledge and its Projections. SOT names the architectural authority role and
remains the prefix for stable decision IDs; it is not the product name.

`MUST` and `MUST NOT` are blocking contracts. `SHOULD` requires a documented
reason to deviate. Illustrative paths, commands, and field names are not frozen
public API.

IDs remain stable even if a decision is later replaced. Downstream design,
Slice contracts, tests, and evidence reference these IDs instead of copying
their rules.
