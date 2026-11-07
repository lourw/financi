-- financi initial schema. See "Data model" in the plan doc.

-- Institution (parent) -> accounts (siblings under it), each with a type.
CREATE TABLE institutions (
  id    INTEGER PRIMARY KEY,
  key   TEXT NOT NULL UNIQUE,            -- matches StatementParser::institution()
  name  TEXT NOT NULL                    -- display name
);
-- Institutions with a parser are built in.
INSERT INTO institutions (key, name) VALUES
  ('amex', 'American Express'),
  ('simplii', 'Simplii Financial');

CREATE TABLE accounts (
  id             INTEGER PRIMARY KEY,
  account_key    TEXT NOT NULL UNIQUE,   -- UUID; used in row_hash and the export manifest
  institution_id INTEGER NOT NULL REFERENCES institutions(id),
  name           TEXT NOT NULL,          -- 'Cobalt', 'No Fee Chequing'
  kind           TEXT NOT NULL CHECK (kind IN ('chequing', 'savings', 'credit_card')),
  currency       TEXT NOT NULL DEFAULT 'CAD',
  UNIQUE (institution_id, name)
);

CREATE TABLE categories (
  id         INTEGER PRIMARY KEY,
  name       TEXT NOT NULL,
  parent_id  INTEGER REFERENCES categories(id),
  kind       TEXT NOT NULL CHECK (kind IN ('spend', 'income', 'transfer')),
  UNIQUE (parent_id, name)
);
-- UNIQUE treats NULLs as distinct, so enforce unique top-level names separately.
CREATE UNIQUE INDEX categories_top_level_name ON categories(name) WHERE parent_id IS NULL;

CREATE TABLE imports (
  id               INTEGER PRIMARY KEY,
  account_id       INTEGER NOT NULL REFERENCES accounts(id),
  parser_id        TEXT NOT NULL,
  file_name        TEXT NOT NULL,
  imported_at      TEXT NOT NULL,
  header_raw       BLOB NOT NULL,
  encoding         TEXT NOT NULL,
  has_bom          INTEGER NOT NULL,
  line_ending      TEXT NOT NULL CHECK (line_ending IN ('lf', 'crlf')),
  trailing_newline INTEGER NOT NULL,
  row_count        INTEGER NOT NULL
);

CREATE TABLE transactions (
  id                    INTEGER PRIMARY KEY,
  account_id            INTEGER NOT NULL REFERENCES accounts(id),
  import_id             INTEGER NOT NULL REFERENCES imports(id) ON DELETE CASCADE,
  source_line           INTEGER NOT NULL,
  raw_bytes             BLOB NOT NULL,
  row_hash              TEXT NOT NULL,
  posted_date           TEXT NOT NULL,
  processed_date        TEXT,
  amount_cents          INTEGER NOT NULL,
  original_currency     TEXT,
  original_amount_cents INTEGER,
  description_raw       TEXT NOT NULL,
  merchant              TEXT,
  category_id           INTEGER REFERENCES categories(id),
  category_locked       INTEGER NOT NULL DEFAULT 0,
  transfer_pair_id      INTEGER REFERENCES transactions(id) ON DELETE SET NULL,
  hidden                INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX tx_reconcile ON transactions(account_id, row_hash);
CREATE INDEX tx_date      ON transactions(posted_date);
CREATE INDEX tx_import    ON transactions(import_id);

CREATE TABLE row_aliases (
  transaction_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
  row_hash       TEXT NOT NULL,
  import_id      INTEGER NOT NULL REFERENCES imports(id) ON DELETE CASCADE,
  PRIMARY KEY (transaction_id, row_hash)
);
CREATE INDEX row_aliases_hash ON row_aliases(row_hash);

CREATE TABLE category_rules (
  id          INTEGER PRIMARY KEY,
  pattern     TEXT NOT NULL,
  category_id INTEGER NOT NULL REFERENCES categories(id) ON DELETE CASCADE,
  priority    INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE flags (
  id             INTEGER PRIMARY KEY,
  transaction_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
  kind           TEXT NOT NULL CHECK (kind IN ('large', 'new_merchant', 'fx_fee')),
  detail         TEXT,
  status         TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'dismissed', 'confirmed')),
  UNIQUE (transaction_id, kind)
);

CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
