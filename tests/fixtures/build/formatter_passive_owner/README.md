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
host output, then separately prints a zero-length static literal and an
explicit null-data, zero-length descriptor. The two empty forms may not
write to the host, and none of the three may allocate or release. An independent scoped runtime array then verifies
one real allocation and release at the original base without relying on
formatter behavior.

The module initializer control binds the formatted `u128` at module
scope, moves it into ordinary print, and then exits through the real host.
It observes one 39-byte allocation, the seven output bytes at its original
base, and one release before the explicit exit.

The checked-consumer control passes the 20-byte rendering of `1.5`
from its 24-byte buffer into ordinary parse, then reads the checked
`f64` result. The original text allocation is released before parse
returns to its caller. The separate checked value remains live through
the read and is released at the caller's scope exit. The observer does
not impose an order between the parser's allocation and text release
inside parse, and distinguishes allocations even if an address is reused.

The user-defined forwarding control returns ordinary format from a
direct wrapper and binds its result by name before printing it. It
checks the original allocation is still live after the wrapper returns,
is used for the real output, and is released once by the consumer.
