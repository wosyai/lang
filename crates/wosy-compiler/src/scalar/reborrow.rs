use super::*;

#[derive(Clone, Copy)]
pub(super) enum AccessAuthority {
    DirectStorage,
    Through(ReferenceOriginId),
}

pub(super) fn loan_authorizes(accessing: &ReferenceLoanId, active: &ReferenceLoanId) -> bool {
    let mut ancestor = Some(accessing);
    while let Some(loan) = ancestor {
        if loan == active {
            return true;
        }
        ancestor = loan.parent.as_deref();
    }
    false
}

pub(super) fn authorizes(
    facts: &[ReferenceOrigin],
    authority: AccessAuthority,
    active: &ReferenceLoanId,
) -> bool {
    match authority {
        AccessAuthority::DirectStorage => false,
        AccessAuthority::Through(origin) => match &facts[origin.0] {
            ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. } => {
                loan_authorizes(loan, active)
            }
            ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => false,
        },
    }
}
