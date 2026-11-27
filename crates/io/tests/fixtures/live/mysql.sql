-- The table crates/io/tests/live_engines.rs reads; safe to run again.
DROP TABLE IF EXISTS ontology_live_item;
DROP TABLE IF EXISTS ontology_live_write;
CREATE TABLE ontology_live_item (
    id      int PRIMARY KEY,
    name    text NOT NULL,
    price   decimal(10, 2) NOT NULL,
    ok      tinyint(1) NOT NULL,
    updated datetime NOT NULL,
    note    text
);
INSERT INTO ontology_live_item VALUES
    (1, 'Pump', 120.50, 1, '2026-01-01 10:00:00', 'first'),
    (2, 'O''Brien & fils — été', 80.00, 0, '2026-02-01 10:00:00', NULL),
    (3, 'Valve', 9.99, 1, '2026-03-01 10:00:00', 'third');
-- What the write tests change.
CREATE TABLE ontology_live_write (
    id    int PRIMARY KEY,
    state varchar(20) NOT NULL
);
INSERT INTO ontology_live_write VALUES
    (1, 'open'), (2, 'open'), (3, 'open');
