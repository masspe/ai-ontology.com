-- The table crates/io/tests/live_engines.rs reads; safe to run again.
DROP TABLE IF EXISTS public.ontology_live_item;
DROP SEQUENCE IF EXISTS public.ontology_live_seq;
CREATE SEQUENCE public.ontology_live_seq;
CREATE TABLE public.ontology_live_item (
    id      integer PRIMARY KEY,
    name    text NOT NULL,
    price   numeric(10, 2) NOT NULL,
    ok      boolean NOT NULL,
    updated timestamp NOT NULL,
    note    text
);
INSERT INTO public.ontology_live_item VALUES
    (1, 'Pump', 120.50, true, '2026-01-01 10:00:00', 'first'),
    (2, 'O''Brien & fils — été', 80.00, false, '2026-02-01 10:00:00', NULL),
    (3, 'Valve', 9.99, true, '2026-03-01 10:00:00', 'third');
