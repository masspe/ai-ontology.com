/* sd/nested.p: a join text sitting inside a doubly-nested comment. Naive,
   non-nesting comment stripping would close at the first "*/" and leak the
   join below as live code; proper nesting must hide it completely. */
/* note: /* draft */ FIND planning WHERE planning.nste EQ sdva.nste NO-LOCK NO-ERROR. */
DISPLAY "sd/nested.p has no live statements".
