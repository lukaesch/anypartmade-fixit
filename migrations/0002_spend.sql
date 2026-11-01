-- Estimated model spend, one row per paid call. Checked against a daily budget.
CREATE TABLE IF NOT EXISTS spend (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  at INTEGER NOT NULL,
  day INTEGER NOT NULL,
  model TEXT NOT NULL,
  session TEXT NOT NULL,
  detail TEXT NOT NULL,
  usd REAL NOT NULL
);
CREATE INDEX IF NOT EXISTS spend_day ON spend (day);
