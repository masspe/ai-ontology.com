-- The table crates/io/tests/live_engines.rs reads; safe to run again.
-- ASCII only (NCHAR for the dash and the accents): what is stored does
-- not depend on the code page of the tool that runs this file.
DROP TABLE IF EXISTS dbo.ontology_live_item;
CREATE TABLE dbo.ontology_live_item (
    id      int PRIMARY KEY,
    name    nvarchar(80) NOT NULL,
    price   decimal(10, 2) NOT NULL,
    ok      bit NOT NULL,
    updated datetime2(0) NOT NULL,
    note    nvarchar(80) NULL
);
INSERT INTO dbo.ontology_live_item VALUES
    (1, N'Pump', 120.50, 1, '2026-01-01T10:00:00', N'first'),
    (2, N'O''Brien & fils ' + NCHAR(8212) + N' ' + NCHAR(233) + N't' + NCHAR(233), 80.00, 0, '2026-02-01T10:00:00', NULL),
    (3, N'Valve', 9.99, 1, '2026-03-01T10:00:00', N'third');
GO
