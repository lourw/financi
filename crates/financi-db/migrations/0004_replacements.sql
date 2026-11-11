-- M4 near-match review (4.3): the old values of transactions replaced by a
-- confirmed "Same charge", so undo_last_import can restore them. The task
-- text said "migration 0003", but 0003_transfers.sql shipped first and
-- migrations are frozen, so this is 0004.
-- 0001_init.sql, 0002_categorize.sql and 0003_transfers.sql are frozen.

-- One row per (import, replaced transaction). The replacement keeps the
-- existing row's id, category and import_id; `import_id` here is the *new*
-- import whose commit overwrote it, so undoing that import restores the
-- old values before the cascade deletes this table and the aliases.
CREATE TABLE replacements (
  import_id INTEGER NOT NULL REFERENCES imports(id) ON DELETE CASCADE,
  transaction_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
  old_raw_bytes BLOB NOT NULL, old_row_hash TEXT NOT NULL,
  old_posted_date TEXT NOT NULL, old_processed_date TEXT,
  old_amount_cents INTEGER NOT NULL, old_description_raw TEXT NOT NULL,
  old_merchant TEXT,
  PRIMARY KEY (import_id, transaction_id)
);
