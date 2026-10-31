-- One JSON document per repair session.
CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY,
  created_at INTEGER NOT NULL,
  data TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_created ON sessions (created_at);

-- Sessions per hashed IP per day.
CREATE TABLE IF NOT EXISTS rate (
  key TEXT PRIMARY KEY,
  day INTEGER NOT NULL,
  n INTEGER NOT NULL
);
