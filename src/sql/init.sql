CREATE TABLE IF NOT EXISTS bean_buyer (
  id              TEXT PRIMARY KEY,
  name            TEXT NOT NULL,
  beans           INTEGER NOT NULL,
  email           TEXT NOT NULL,
  ssn             TEXT NOT NULL,
  bean_stats      INTEGER NOT NULL
)
