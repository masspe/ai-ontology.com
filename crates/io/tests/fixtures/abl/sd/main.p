/* sd/main.p: buffer alias join across lines, EQ and =, plus the implicit
   FOR EACH ... OF ... join on the same pair of tables. */
DEFINE BUFFER b-sdva FOR sdva.

FIND planning WHERE planning.nste EQ b-sdva.nste
              AND   planning.peri = b-sdva.peri
    NO-LOCK NO-ERROR.

FOR EACH planning OF sdva:
    END.
