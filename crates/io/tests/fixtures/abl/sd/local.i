/* sd/local.i: a temp-table that shadows a real table name, and a join
   against a table the schema does not know. Neither should produce a
   joined_on pair. */
DEFINE TEMP-TABLE planning NO-UNDO FIELD nste AS INTEGER.
FOR EACH planning OF sdva:
    END.
FIND madeuptbl WHERE madeuptbl.id EQ sdva.id NO-LOCK NO-ERROR.
