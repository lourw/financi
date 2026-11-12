-- 3.73: indexes so imports, undo and categorizing stay fast as data grows.
-- 0001_init.sql .. 0004_replacements.sql are frozen.

-- The self-referencing `transfer_pair_id` foreign key (0001) resolves and
-- NULLs links on delete by scanning the whole table once per deleted row —
-- quadratic undo. Partial because only linked rows matter.
CREATE INDEX tx_transfer_pair ON transactions(transfer_pair_id)
  WHERE transfer_pair_id IS NOT NULL;

-- Category reassign, per-category counts and the table's category filter
-- all look transactions up by category (3.54 no longer needs to add it).
CREATE INDEX tx_category ON transactions(category_id);

-- The transactions table's default order (posted date, then import, then
-- source line); covers date-range scans and the newest-first page walk.
CREATE INDEX tx_date_import_line ON transactions(posted_date, import_id, source_line);
