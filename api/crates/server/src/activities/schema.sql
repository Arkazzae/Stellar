CREATE TABLE IF NOT EXISTS ActivityInstalls (
    Id TEXT PRIMARY KEY, Source TEXT NOT NULL, Activity TEXT NOT NULL,
    Digest TEXT NOT NULL, Enabled INTEGER NOT NULL, UNIQUE(Source, Activity)
);
CREATE TABLE IF NOT EXISTS ActivityVersions (
    Install TEXT NOT NULL REFERENCES ActivityInstalls(Id) ON DELETE CASCADE,
    Digest TEXT NOT NULL, Revision TEXT NOT NULL, Release TEXT NOT NULL,
    Manifest TEXT NOT NULL, Bytes INTEGER NOT NULL,
    PRIMARY KEY (Install, Digest)
);
CREATE TABLE IF NOT EXISTS ActivityFiles (
    Install TEXT NOT NULL, Digest TEXT NOT NULL, Path TEXT NOT NULL, Content BLOB NOT NULL,
    PRIMARY KEY (Install, Digest, Path),
    FOREIGN KEY (Install, Digest) REFERENCES ActivityVersions(Install, Digest) ON DELETE CASCADE
);
