/* pr/local.i: a temp-table that shadows a real table name, and a join
   against a table the schema does not know. Neither should produce a
   joined_on pair. */
DEFINE TEMP-TABLE schedule NO-UNDO FIELD cono AS INTEGER.
FOR EACH schedule OF payvar:
    END.
FIND madeuptbl WHERE madeuptbl.id EQ payvar.id NO-LOCK NO-ERROR.
