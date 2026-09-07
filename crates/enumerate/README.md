# merc_enumerate

Enumerates ground constructor instances of a (possibly infinite) data sort.

Neither `merc_sabre` nor `merc_explore` can close an open data expression: pick
concrete constructor instances for a set of free variables of a sort. This
crate owns that single mechanism, which is needed both to eliminate `exists`/
`forall` quantifiers during rewriting and to instantiate `sum d:D` variables
while generating LPS successors — see `docs/enumeration-crate-plan.md` in the
workspace root for the full design.

Enumeration always proceeds over a sort's *constructors*, normalising each
candidate with a `merc_sabre::RewriteEngine` rather than ever descending into
a native machine-word representation.
