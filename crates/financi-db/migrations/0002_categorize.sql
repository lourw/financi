-- M2 categorize: regex rules and system categories.
-- 0001_init.sql is frozen; all schema changes since then live here.

ALTER TABLE category_rules ADD COLUMN is_regex INTEGER NOT NULL DEFAULT 0;

ALTER TABLE categories ADD COLUMN is_system INTEGER NOT NULL DEFAULT 0;

-- Built-in transfer categories, present in every database. No other
-- categories are created by default.
INSERT INTO categories (name, parent_id, kind, is_system) VALUES
  ('Transfers', NULL, 'transfer', 1);
INSERT INTO categories (name, parent_id, kind, is_system) VALUES
  ('Card Payment',
   (SELECT id FROM categories WHERE name = 'Transfers' AND parent_id IS NULL),
   'transfer', 1),
  ('Between Accounts',
   (SELECT id FROM categories WHERE name = 'Transfers' AND parent_id IS NULL),
   'transfer', 1);
