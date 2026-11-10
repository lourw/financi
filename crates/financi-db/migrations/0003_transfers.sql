-- M4 transfer pairing (4.2): reviews for ambiguous pairings, and the
-- exclusion marker that keeps "Not a transfer" rows out of later runs.
-- 0001_init.sql and 0002_categorize.sql are frozen.

-- One row per ambiguous transaction: the row under review and the ids of
-- every candidate it could pair with, comma-separated in file order of
-- posted date then id. Resolving a review deletes this row; every pairing
-- run recomputes the table from scratch.
CREATE TABLE transfer_reviews (
  transaction_id INTEGER PRIMARY KEY REFERENCES transactions(id) ON DELETE CASCADE,
  candidate_ids  TEXT NOT NULL  -- comma-separated transaction ids
);

-- Rows the user opted out of pairing ("Not a transfer", or one side of an
-- unpair): the pairing engine never considers them again. This mirrors the
-- manual-edit lock (`category_locked`) on the categorization side — a human
-- decision the automatic engine must never override.
ALTER TABLE transactions ADD COLUMN transfer_excluded INTEGER NOT NULL DEFAULT 0;
