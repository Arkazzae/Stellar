-- What the last look for a newer release of an installed activity found, so
-- that Cockpit shows it at once and the `activity-updates` worker knows when
-- to look again. `Digest` is the version that was installed when it looked:
-- a finding about a version since replaced is ignored. `Version` and
-- `Release` name a newer release and are null when there was none; `Error`
-- says why the look failed.
CREATE TABLE IF NOT EXISTS ActivityUpdateChecks (
    Install TEXT PRIMARY KEY REFERENCES ActivityInstalls(Id) ON DELETE CASCADE,
    Digest TEXT NOT NULL, CheckedAt TEXT NOT NULL, NextAt TEXT NOT NULL,
    Version TEXT, Release TEXT, Error TEXT
);
