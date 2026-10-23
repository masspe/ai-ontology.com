/* pr/main.p: buffer alias join across lines, EQ and =, plus the implicit
   FOR EACH ... OF ... join on the same pair of tables. */
DEFINE BUFFER b-payvar FOR payvar.

FIND schedule WHERE schedule.cono EQ b-payvar.cono
              AND   schedule.period = b-payvar.period
    NO-LOCK NO-ERROR.

FOR EACH schedule OF payvar:
    END.
