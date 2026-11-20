-- Your SQL goes here
ALTER TABLE volumes ADD COLUMN reclaim_policy INT NOT NULL DEFAULT 0;
