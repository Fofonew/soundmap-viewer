# ADR 0004 — MNH calculé localement (MNS − MNT)

**Statut :** accepté (session 1)

Seuls le MNT et le MNS sont téléchargés. Le MNH est calculé à la finalisation : `max(0, MNS − MNT)` pixel à pixel, nodata si l'un des deux manque.

Cela économise un tiers du volume de relief, soit 96 Mo sur une zone de 5 km. C'est équivalent au produit IGN, qui est lui-même défini comme MNS − MNT. La couche WMS-R `…MNH…` reste disponible dans `sm-data::wmsr` pour vérification ponctuelle.
