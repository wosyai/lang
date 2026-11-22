# Passive formatter ownership witness

This fixture reconstructs the formatter scenarios behaviorally; it does not
contain the historical probe sources or replace their separate evidence.

The nested case forwards the actual print request and checks the request's
base, bytes, and release window. In the named case, markers carry literal
IDs only. Wosy independently loads bytes through each still-live named
descriptor and checks its length, then rereads the outer descriptor after
the inner scope ends. The observer checks full contents directly at the
bases returned by the real allocator, and verifies that those same bases
are freed once each in inner-then-outer order. The named observer does
not receive or compare descriptor pointers at the ABI boundary.

The separate controls executable prints a static literal through real
host output, then prints an empty descriptor (no host write). Neither may
allocate or release. An independent scoped runtime array then verifies
one real allocation and release at the original base without relying on
formatter behavior.
