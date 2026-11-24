define i128 @__multi3(i128 %a, i128 %b) {
entry:
  %a.slot = alloca i128, align 16
  store i128 %a, ptr %a.slot, align 16
  %a.lo.ptr = getelementptr i64, ptr %a.slot, i32 0
  %a0 = load i64, ptr %a.lo.ptr, align 8
  %a.hi.ptr = getelementptr i64, ptr %a.slot, i32 1
  %a1 = load i64, ptr %a.hi.ptr, align 8
  %b.slot = alloca i128, align 16
  store i128 %b, ptr %b.slot, align 16
  %b.lo.ptr = getelementptr i64, ptr %b.slot, i32 0
  %b0 = load i64, ptr %b.lo.ptr, align 8
  %b.hi.ptr = getelementptr i64, ptr %b.slot, i32 1
  %b1 = load i64, ptr %b.hi.ptr, align 8
  %a0.lo = and i64 %a0, 4294967295
  %a0.hi = lshr i64 %a0, 32
  %b0.lo = and i64 %b0, 4294967295
  %b0.hi = lshr i64 %b0, 32
  %p.ll = mul i64 %a0.lo, %b0.lo
  %p.lh = mul i64 %a0.lo, %b0.hi
  %p.hl = mul i64 %a0.hi, %b0.lo
  %p.hh = mul i64 %a0.hi, %b0.hi
  %p.lh.lo = and i64 %p.lh, 4294967295
  %p.hl.lo = and i64 %p.hl, 4294967295
  %p.ll.hi = lshr i64 %p.ll, 32
  %t0 = add i64 %p.ll.hi, %p.lh.lo
  %c0 = icmp ult i64 %t0, %p.ll.hi
  %c0.ext = zext i1 %c0 to i64
  %t1 = add i64 %t0, %p.hl.lo
  %c1 = icmp ult i64 %t1, %t0
  %c1.ext = zext i1 %c1 to i64
  %p.lh.hi = lshr i64 %p.lh, 32
  %p.hl.hi = lshr i64 %p.hl, 32
  %t1.hi = lshr i64 %t1, 32
  %hi0.a = add i64 %p.hh, %p.lh.hi
  %hi0.b = add i64 %hi0.a, %p.hl.hi
  %hi0.c = add i64 %hi0.b, %t1.hi
  %hi0.d = add i64 %hi0.c, %c0.ext
  %hi0 = add i64 %hi0.d, %c1.ext
  %lo0 = mul i64 %a0, %b0
  %t.a0b1 = mul i64 %a0, %b1
  %t.a1b0 = mul i64 %a1, %b0
  %hi.a = add i64 %hi0, %t.a0b1
  %hi = add i64 %hi.a, %t.a1b0
  %r.slot = alloca i128, align 16
  %r.lo.ptr = getelementptr i64, ptr %r.slot, i32 0
  store i64 %lo0, ptr %r.lo.ptr, align 8
  %r.hi.ptr = getelementptr i64, ptr %r.slot, i32 1
  store i64 %hi, ptr %r.hi.ptr, align 8
  %r = load i128, ptr %r.slot, align 16
  ret i128 %r
}

define i128 @__udivti3(i128 %a, i128 %b) {
entry:
  %a.slot = alloca i128, align 16
  store i128 %a, ptr %a.slot, align 16
  %a.lo.ptr = getelementptr i64, ptr %a.slot, i32 0
  %a0 = load i64, ptr %a.lo.ptr, align 8
  %a.hi.ptr = getelementptr i64, ptr %a.slot, i32 1
  %a1 = load i64, ptr %a.hi.ptr, align 8
  %b.slot = alloca i128, align 16
  store i128 %b, ptr %b.slot, align 16
  %b.lo.ptr = getelementptr i64, ptr %b.slot, i32 0
  %b0 = load i64, ptr %b.lo.ptr, align 8
  %b.hi.ptr = getelementptr i64, ptr %b.slot, i32 1
  %b1 = load i64, ptr %b.hi.ptr, align 8
  %b.lo.zero = icmp eq i64 %b0, 0
  %b.hi.zero = icmp eq i64 %b1, 0
  %b.is.zero = and i1 %b.lo.zero, %b.hi.zero
  br i1 %b.is.zero, label %div.zero, label %loop

div.zero:
  ret i128 -1

loop:
  %i = phi i64 [ 127, %entry ], [ %i.next, %loop ]
  %q1 = phi i64 [ 0, %entry ], [ %q1.next, %loop ]
  %q0 = phi i64 [ 0, %entry ], [ %q0.next, %loop ]
  %r1 = phi i64 [ 0, %entry ], [ %r1.next, %loop ]
  %r0 = phi i64 [ 0, %entry ], [ %r0.next, %loop ]
  %i.ge.64 = icmp uge i64 %i, 64
  %i.sub.64 = sub i64 %i, 64
  %a1.bit.raw = lshr i64 %a1, %i.sub.64
  %a0.bit.raw = lshr i64 %a0, %i
  %bit.raw = select i1 %i.ge.64, i64 %a1.bit.raw, i64 %a0.bit.raw
  %bit = and i64 %bit.raw, 1
  %r0.carry = lshr i64 %r0, 63
  %r0.sh = shl i64 %r0, 1
  %r0.acc = or i64 %r0.sh, %bit
  %r1.sh = shl i64 %r1, 1
  %r1.acc = or i64 %r1.sh, %r0.carry
  %r.hi.gt = icmp ugt i64 %r1.acc, %b1
  %r.hi.eq = icmp eq i64 %r1.acc, %b1
  %r.lo.ge = icmp uge i64 %r0.acc, %b0
  %r.lo.take = and i1 %r.hi.eq, %r.lo.ge
  %r.ge = or i1 %r.hi.gt, %r.lo.take
  %sub0 = sub i64 %r0.acc, %b0
  %borrow = icmp ult i64 %r0.acc, %b0
  %borrow.ext = zext i1 %borrow to i64
  %sub1.tmp = sub i64 %r1.acc, %b1
  %sub1 = sub i64 %sub1.tmp, %borrow.ext
  %r0.next = select i1 %r.ge, i64 %sub0, i64 %r0.acc
  %r1.next = select i1 %r.ge, i64 %sub1, i64 %r1.acc
  %qbit = select i1 %r.ge, i64 1, i64 0
  %q.sh.hi = shl i64 %qbit, %i.sub.64
  %q.sh.lo = shl i64 %qbit, %i
  %q1.or = or i64 %q1, %q.sh.hi
  %q0.or = or i64 %q0, %q.sh.lo
  %q1.next = select i1 %i.ge.64, i64 %q1.or, i64 %q1
  %q0.next = select i1 %i.ge.64, i64 %q0, i64 %q0.or
  %last = icmp eq i64 %i, 0
  %i.next = sub i64 %i, 1
  br i1 %last, label %exit, label %loop

exit:
  %q1.fin = phi i64 [ %q1.next, %loop ]
  %q0.fin = phi i64 [ %q0.next, %loop ]
  %r.slot = alloca i128, align 16
  %r.lo.ptr = getelementptr i64, ptr %r.slot, i32 0
  store i64 %q0.fin, ptr %r.lo.ptr, align 8
  %r.hi.ptr = getelementptr i64, ptr %r.slot, i32 1
  store i64 %q1.fin, ptr %r.hi.ptr, align 8
  %r = load i128, ptr %r.slot, align 16
  ret i128 %r
}

define i128 @__divti3(i128 %a, i128 %b) {
entry:
  %a.slot = alloca i128, align 16
  store i128 %a, ptr %a.slot, align 16
  %a.lo.ptr = getelementptr i64, ptr %a.slot, i32 0
  %a0 = load i64, ptr %a.lo.ptr, align 8
  %a.hi.ptr = getelementptr i64, ptr %a.slot, i32 1
  %a1 = load i64, ptr %a.hi.ptr, align 8
  %b.slot = alloca i128, align 16
  store i128 %b, ptr %b.slot, align 16
  %b.lo.ptr = getelementptr i64, ptr %b.slot, i32 0
  %b0 = load i64, ptr %b.lo.ptr, align 8
  %b.hi.ptr = getelementptr i64, ptr %b.slot, i32 1
  %b1 = load i64, ptr %b.hi.ptr, align 8
  %a.neg = icmp slt i64 %a1, 0
  %b.neg = icmp slt i64 %b1, 0
  %a0.nz = icmp ne i64 %a0, 0
  %a0.nz.ext = zext i1 %a0.nz to i64
  %na0 = sub i64 0, %a0
  %na1.tmp = sub i64 0, %a1
  %na1 = sub i64 %na1.tmp, %a0.nz.ext
  %ma0 = select i1 %a.neg, i64 %na0, i64 %a0
  %ma1 = select i1 %a.neg, i64 %na1, i64 %a1
  %b0.nz = icmp ne i64 %b0, 0
  %b0.nz.ext = zext i1 %b0.nz to i64
  %nb0 = sub i64 0, %b0
  %nb1.tmp = sub i64 0, %b1
  %nb1 = sub i64 %nb1.tmp, %b0.nz.ext
  %mb0 = select i1 %b.neg, i64 %nb0, i64 %b0
  %mb1 = select i1 %b.neg, i64 %nb1, i64 %b1
  %ma.slot = alloca i128, align 16
  %ma.lo.ptr = getelementptr i64, ptr %ma.slot, i32 0
  store i64 %ma0, ptr %ma.lo.ptr, align 8
  %ma.hi.ptr = getelementptr i64, ptr %ma.slot, i32 1
  store i64 %ma1, ptr %ma.hi.ptr, align 8
  %ma = load i128, ptr %ma.slot, align 16
  %mb.slot = alloca i128, align 16
  %mb.lo.ptr = getelementptr i64, ptr %mb.slot, i32 0
  store i64 %mb0, ptr %mb.lo.ptr, align 8
  %mb.hi.ptr = getelementptr i64, ptr %mb.slot, i32 1
  store i64 %mb1, ptr %mb.hi.ptr, align 8
  %mb = load i128, ptr %mb.slot, align 16
  %q.mag = call i128 @__udivti3(i128 %ma, i128 %mb)
  %q.slot = alloca i128, align 16
  store i128 %q.mag, ptr %q.slot, align 16
  %q.lo.ptr = getelementptr i64, ptr %q.slot, i32 0
  %q0 = load i64, ptr %q.lo.ptr, align 8
  %q.hi.ptr = getelementptr i64, ptr %q.slot, i32 1
  %q1 = load i64, ptr %q.hi.ptr, align 8
  %q.neg = xor i1 %a.neg, %b.neg
  %q0.nz = icmp ne i64 %q0, 0
  %q0.nz.ext = zext i1 %q0.nz to i64
  %nq0 = sub i64 0, %q0
  %nq1.tmp = sub i64 0, %q1
  %nq1 = sub i64 %nq1.tmp, %q0.nz.ext
  %r0 = select i1 %q.neg, i64 %nq0, i64 %q0
  %r1 = select i1 %q.neg, i64 %nq1, i64 %q1
  %r.slot = alloca i128, align 16
  %r.lo.ptr = getelementptr i64, ptr %r.slot, i32 0
  store i64 %r0, ptr %r.lo.ptr, align 8
  %r.hi.ptr = getelementptr i64, ptr %r.slot, i32 1
  store i64 %r1, ptr %r.hi.ptr, align 8
  %r = load i128, ptr %r.slot, align 16
  ret i128 %r
}

define i128 @__umodti3(i128 %a, i128 %b) {
entry:
  %a.slot = alloca i128, align 16
  store i128 %a, ptr %a.slot, align 16
  %a.lo.ptr = getelementptr i64, ptr %a.slot, i32 0
  %a0 = load i64, ptr %a.lo.ptr, align 8
  %a.hi.ptr = getelementptr i64, ptr %a.slot, i32 1
  %a1 = load i64, ptr %a.hi.ptr, align 8
  %b.slot = alloca i128, align 16
  store i128 %b, ptr %b.slot, align 16
  %b.lo.ptr = getelementptr i64, ptr %b.slot, i32 0
  %b0 = load i64, ptr %b.lo.ptr, align 8
  %b.hi.ptr = getelementptr i64, ptr %b.slot, i32 1
  %b1 = load i64, ptr %b.hi.ptr, align 8
  %b.lo.zero = icmp eq i64 %b0, 0
  %b.hi.zero = icmp eq i64 %b1, 0
  %b.is.zero = and i1 %b.lo.zero, %b.hi.zero
  br i1 %b.is.zero, label %mod.zero, label %loop

mod.zero:
  ret i128 %a

loop:
  %i = phi i64 [ 127, %entry ], [ %i.next, %loop ]
  %q1 = phi i64 [ 0, %entry ], [ %q1.next, %loop ]
  %q0 = phi i64 [ 0, %entry ], [ %q0.next, %loop ]
  %r1 = phi i64 [ 0, %entry ], [ %r1.next, %loop ]
  %r0 = phi i64 [ 0, %entry ], [ %r0.next, %loop ]
  %i.ge.64 = icmp uge i64 %i, 64
  %i.sub.64 = sub i64 %i, 64
  %a1.bit.raw = lshr i64 %a1, %i.sub.64
  %a0.bit.raw = lshr i64 %a0, %i
  %bit.raw = select i1 %i.ge.64, i64 %a1.bit.raw, i64 %a0.bit.raw
  %bit = and i64 %bit.raw, 1
  %r0.carry = lshr i64 %r0, 63
  %r0.sh = shl i64 %r0, 1
  %r0.acc = or i64 %r0.sh, %bit
  %r1.sh = shl i64 %r1, 1
  %r1.acc = or i64 %r1.sh, %r0.carry
  %r.hi.gt = icmp ugt i64 %r1.acc, %b1
  %r.hi.eq = icmp eq i64 %r1.acc, %b1
  %r.lo.ge = icmp uge i64 %r0.acc, %b0
  %r.lo.take = and i1 %r.hi.eq, %r.lo.ge
  %r.ge = or i1 %r.hi.gt, %r.lo.take
  %sub0 = sub i64 %r0.acc, %b0
  %borrow = icmp ult i64 %r0.acc, %b0
  %borrow.ext = zext i1 %borrow to i64
  %sub1.tmp = sub i64 %r1.acc, %b1
  %sub1 = sub i64 %sub1.tmp, %borrow.ext
  %r0.next = select i1 %r.ge, i64 %sub0, i64 %r0.acc
  %r1.next = select i1 %r.ge, i64 %sub1, i64 %r1.acc
  %qbit = select i1 %r.ge, i64 1, i64 0
  %q.sh.hi = shl i64 %qbit, %i.sub.64
  %q.sh.lo = shl i64 %qbit, %i
  %q1.or = or i64 %q1, %q.sh.hi
  %q0.or = or i64 %q0, %q.sh.lo
  %q1.next = select i1 %i.ge.64, i64 %q1.or, i64 %q1
  %q0.next = select i1 %i.ge.64, i64 %q0, i64 %q0.or
  %last = icmp eq i64 %i, 0
  %i.next = sub i64 %i, 1
  br i1 %last, label %exit, label %loop

exit:
  %r0.fin = phi i64 [ %r0.next, %loop ]
  %r1.fin = phi i64 [ %r1.next, %loop ]
  %r.slot = alloca i128, align 16
  %r.lo.ptr = getelementptr i64, ptr %r.slot, i32 0
  store i64 %r0.fin, ptr %r.lo.ptr, align 8
  %r.hi.ptr = getelementptr i64, ptr %r.slot, i32 1
  store i64 %r1.fin, ptr %r.hi.ptr, align 8
  %r = load i128, ptr %r.slot, align 16
  ret i128 %r
}

define i128 @__modti3(i128 %a, i128 %b) {
entry:
  %a.slot = alloca i128, align 16
  store i128 %a, ptr %a.slot, align 16
  %a.lo.ptr = getelementptr i64, ptr %a.slot, i32 0
  %a0 = load i64, ptr %a.lo.ptr, align 8
  %a.hi.ptr = getelementptr i64, ptr %a.slot, i32 1
  %a1 = load i64, ptr %a.hi.ptr, align 8
  %b.slot = alloca i128, align 16
  store i128 %b, ptr %b.slot, align 16
  %b.lo.ptr = getelementptr i64, ptr %b.slot, i32 0
  %b0 = load i64, ptr %b.lo.ptr, align 8
  %b.hi.ptr = getelementptr i64, ptr %b.slot, i32 1
  %b1 = load i64, ptr %b.hi.ptr, align 8
  %a.neg = icmp slt i64 %a1, 0
  %b.neg = icmp slt i64 %b1, 0
  %a0.nz = icmp ne i64 %a0, 0
  %a0.nz.ext = zext i1 %a0.nz to i64
  %na0 = sub i64 0, %a0
  %na1.tmp = sub i64 0, %a1
  %na1 = sub i64 %na1.tmp, %a0.nz.ext
  %ma0 = select i1 %a.neg, i64 %na0, i64 %a0
  %ma1 = select i1 %a.neg, i64 %na1, i64 %a1
  %b0.nz = icmp ne i64 %b0, 0
  %b0.nz.ext = zext i1 %b0.nz to i64
  %nb0 = sub i64 0, %b0
  %nb1.tmp = sub i64 0, %b1
  %nb1 = sub i64 %nb1.tmp, %b0.nz.ext
  %mb0 = select i1 %b.neg, i64 %nb0, i64 %b0
  %mb1 = select i1 %b.neg, i64 %nb1, i64 %b1
  %ma.slot = alloca i128, align 16
  %ma.lo.ptr = getelementptr i64, ptr %ma.slot, i32 0
  store i64 %ma0, ptr %ma.lo.ptr, align 8
  %ma.hi.ptr = getelementptr i64, ptr %ma.slot, i32 1
  store i64 %ma1, ptr %ma.hi.ptr, align 8
  %ma = load i128, ptr %ma.slot, align 16
  %mb.slot = alloca i128, align 16
  %mb.lo.ptr = getelementptr i64, ptr %mb.slot, i32 0
  store i64 %mb0, ptr %mb.lo.ptr, align 8
  %mb.hi.ptr = getelementptr i64, ptr %mb.slot, i32 1
  store i64 %mb1, ptr %mb.hi.ptr, align 8
  %mb = load i128, ptr %mb.slot, align 16
  %r.mag = call i128 @__umodti3(i128 %ma, i128 %mb)
  %m.slot = alloca i128, align 16
  store i128 %r.mag, ptr %m.slot, align 16
  %m.lo.ptr = getelementptr i64, ptr %m.slot, i32 0
  %m0 = load i64, ptr %m.lo.ptr, align 8
  %m.hi.ptr = getelementptr i64, ptr %m.slot, i32 1
  %m1 = load i64, ptr %m.hi.ptr, align 8
  %m0.nz = icmp ne i64 %m0, 0
  %m0.nz.ext = zext i1 %m0.nz to i64
  %nm0 = sub i64 0, %m0
  %nm1.tmp = sub i64 0, %m1
  %nm1 = sub i64 %nm1.tmp, %m0.nz.ext
  %r0 = select i1 %a.neg, i64 %nm0, i64 %m0
  %r1 = select i1 %a.neg, i64 %nm1, i64 %m1
  %r.slot = alloca i128, align 16
  %r.lo.ptr = getelementptr i64, ptr %r.slot, i32 0
  store i64 %r0, ptr %r.lo.ptr, align 8
  %r.hi.ptr = getelementptr i64, ptr %r.slot, i32 1
  store i64 %r1, ptr %r.hi.ptr, align 8
  %r = load i128, ptr %r.slot, align 16
  ret i128 %r
}

define i128 @__ashlti3(i128 %a, i128 %b) {
entry:
  %a.slot = alloca i128, align 16
  store i128 %a, ptr %a.slot, align 16
  %a.lo.ptr = getelementptr i64, ptr %a.slot, i32 0
  %a0 = load i64, ptr %a.lo.ptr, align 8
  %a.hi.ptr = getelementptr i64, ptr %a.slot, i32 1
  %a1 = load i64, ptr %a.hi.ptr, align 8
  %b.slot = alloca i128, align 16
  store i128 %b, ptr %b.slot, align 16
  %b.lo.ptr = getelementptr i64, ptr %b.slot, i32 0
  %b0 = load i64, ptr %b.lo.ptr, align 8
  %n = and i64 %b0, 127
  %n.is.zero = icmp eq i64 %n, 0
  br i1 %n.is.zero, label %sh.identity, label %sh.split

sh.identity:
  ret i128 %a

sh.split:
  %n.ge.64 = icmp uge i64 %n, 64
  br i1 %n.ge.64, label %sh.big, label %sh.small

sh.small:
  %s.lo = shl i64 %a0, %n
  %s.comp = sub i64 64, %n
  %s.carry = lshr i64 %a0, %s.comp
  %s.hi.sh = shl i64 %a1, %n
  %s.hi = or i64 %s.hi.sh, %s.carry
  br label %sh.exit

sh.big:
  %b.sh = sub i64 %n, 64
  %b.hi = shl i64 %a0, %b.sh
  br label %sh.exit

sh.exit:
  %r.lo = phi i64 [ %s.lo, %sh.small ], [ 0, %sh.big ]
  %r.hi = phi i64 [ %s.hi, %sh.small ], [ %b.hi, %sh.big ]
  %r.slot = alloca i128, align 16
  %r.lo.ptr = getelementptr i64, ptr %r.slot, i32 0
  store i64 %r.lo, ptr %r.lo.ptr, align 8
  %r.hi.ptr = getelementptr i64, ptr %r.slot, i32 1
  store i64 %r.hi, ptr %r.hi.ptr, align 8
  %r = load i128, ptr %r.slot, align 16
  ret i128 %r
}

define i128 @__lshrti3(i128 %a, i128 %b) {
entry:
  %a.slot = alloca i128, align 16
  store i128 %a, ptr %a.slot, align 16
  %a.lo.ptr = getelementptr i64, ptr %a.slot, i32 0
  %a0 = load i64, ptr %a.lo.ptr, align 8
  %a.hi.ptr = getelementptr i64, ptr %a.slot, i32 1
  %a1 = load i64, ptr %a.hi.ptr, align 8
  %b.slot = alloca i128, align 16
  store i128 %b, ptr %b.slot, align 16
  %b.lo.ptr = getelementptr i64, ptr %b.slot, i32 0
  %b0 = load i64, ptr %b.lo.ptr, align 8
  %n = and i64 %b0, 127
  %n.is.zero = icmp eq i64 %n, 0
  br i1 %n.is.zero, label %sh.identity, label %sh.split

sh.identity:
  ret i128 %a

sh.split:
  %n.ge.64 = icmp uge i64 %n, 64
  br i1 %n.ge.64, label %sh.big, label %sh.small

sh.small:
  %s.hi = lshr i64 %a1, %n
  %s.comp = sub i64 64, %n
  %s.carry = shl i64 %a1, %s.comp
  %s.lo.sh = lshr i64 %a0, %n
  %s.lo = or i64 %s.lo.sh, %s.carry
  br label %sh.exit

sh.big:
  %b.sh = sub i64 %n, 64
  %b.lo = lshr i64 %a1, %b.sh
  br label %sh.exit

sh.exit:
  %r.lo = phi i64 [ %s.lo, %sh.small ], [ %b.lo, %sh.big ]
  %r.hi = phi i64 [ %s.hi, %sh.small ], [ 0, %sh.big ]
  %r.slot = alloca i128, align 16
  %r.lo.ptr = getelementptr i64, ptr %r.slot, i32 0
  store i64 %r.lo, ptr %r.lo.ptr, align 8
  %r.hi.ptr = getelementptr i64, ptr %r.slot, i32 1
  store i64 %r.hi, ptr %r.hi.ptr, align 8
  %r = load i128, ptr %r.slot, align 16
  ret i128 %r
}

define i128 @__ashrti3(i128 %a, i128 %b) {
entry:
  %a.slot = alloca i128, align 16
  store i128 %a, ptr %a.slot, align 16
  %a.lo.ptr = getelementptr i64, ptr %a.slot, i32 0
  %a0 = load i64, ptr %a.lo.ptr, align 8
  %a.hi.ptr = getelementptr i64, ptr %a.slot, i32 1
  %a1 = load i64, ptr %a.hi.ptr, align 8
  %b.slot = alloca i128, align 16
  store i128 %b, ptr %b.slot, align 16
  %b.lo.ptr = getelementptr i64, ptr %b.slot, i32 0
  %b0 = load i64, ptr %b.lo.ptr, align 8
  %n = and i64 %b0, 127
  %n.is.zero = icmp eq i64 %n, 0
  br i1 %n.is.zero, label %sh.identity, label %sh.split

sh.identity:
  ret i128 %a

sh.split:
  %n.ge.64 = icmp uge i64 %n, 64
  br i1 %n.ge.64, label %sh.big, label %sh.small

sh.small:
  %s.hi = ashr i64 %a1, %n
  %s.comp = sub i64 64, %n
  %s.carry = shl i64 %a1, %s.comp
  %s.lo.sh = lshr i64 %a0, %n
  %s.lo = or i64 %s.lo.sh, %s.carry
  br label %sh.exit

sh.big:
  %b.sh = sub i64 %n, 64
  %b.lo = ashr i64 %a1, %b.sh
  %b.hi = ashr i64 %a1, 63
  br label %sh.exit

sh.exit:
  %r.lo = phi i64 [ %s.lo, %sh.small ], [ %b.lo, %sh.big ]
  %r.hi = phi i64 [ %s.hi, %sh.small ], [ %b.hi, %sh.big ]
  %r.slot = alloca i128, align 16
  %r.lo.ptr = getelementptr i64, ptr %r.slot, i32 0
  store i64 %r.lo, ptr %r.lo.ptr, align 8
  %r.hi.ptr = getelementptr i64, ptr %r.slot, i32 1
  store i64 %r.hi, ptr %r.hi.ptr, align 8
  %r = load i128, ptr %r.slot, align 16
  ret i128 %r
}
