-- 3.68: a confirmed "Same charge" replacement overwrites the transaction's
-- original_currency / original_amount_cents too, so `replacements` must keep
-- their old values for undo to restore them. Rows replaced before this
-- change keep NULL columns: undoing one leaves the FX columns as the
-- replacement set them (nothing older is recorded).
-- 0001_init.sql … 0005_perf_indexes.sql are frozen.

ALTER TABLE replacements ADD COLUMN old_original_currency TEXT;
ALTER TABLE replacements ADD COLUMN old_original_amount_cents INTEGER;
