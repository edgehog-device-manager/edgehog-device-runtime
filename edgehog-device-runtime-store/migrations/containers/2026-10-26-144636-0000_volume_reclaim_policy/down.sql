-- This file should undo anything in `up.sql`
ALTER TABLE volumes DROP COLUMN reclaim_policy;
