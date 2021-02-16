use crate::context::LintContext;
use crate::rustc_middle::ty::TypeFoldable;
use crate::LateContext;
use crate::LateLintPass;
use rustc_hir::def::DefKind;
use rustc_hir::{Expr, ExprKind};
use rustc_middle::ty;
use rustc_span::symbol::sym;

declare_lint! {
    /// The `noop_method_call` lint detects specific calls to noop methods
    /// such as a calling `<&T as Clone>::clone` where `T: !Clone`.
    ///
    /// ### Example
    ///
    /// ```rust
    /// # #![allow(unused)]
    /// struct Foo;
    /// let foo = &Foo;
    /// let clone: &Foo = foo.clone();
    /// ```
    ///
    /// {{produces}}
    ///
    /// ### Explanation
    ///
    /// Some method calls are noops meaning that they do nothing. Usually such methods
    /// are the result of blanket implementations that happen to create some method invocations
    /// that end up not doing anything. For instance, `Clone` is implemented on all `&T`, but
    /// calling `clone` on a `&T` where `T` does not implement clone, actually doesn't do anything
    /// as references are copy. This lint detects these calls and warns the user about them.
    pub NOOP_METHOD_CALL,
    Warn,
    "detects the use of well-known noop methods"
}

declare_lint_pass!(NoopMethodCall => [NOOP_METHOD_CALL]);

impl<'tcx> LateLintPass<'tcx> for NoopMethodCall {
    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'_>) {
        // We only care about method calls.
        let (call, elements) = match expr.kind {
            ExprKind::MethodCall(call, _, elements, _) => (call, elements),
            _ => return,
        };
        // We only care about method calls corresponding to the `Clone`, `Deref` and `Borrow`
        // traits and ignore any other method call.
        let (trait_id, did) = match cx.typeck_results().type_dependent_def(expr.hir_id) {
            // Verify we are dealing with a method/associated function.
            Some((DefKind::AssocFn, did)) => match cx.tcx.trait_of_item(did) {
                // Check that we're dealing with a trait method for one of the traits we care about.
                Some(trait_id)
                    if [sym::Clone].iter().any(|s| cx.tcx.is_diagnostic_item(*s, trait_id)) =>
                {
                    (trait_id, did)
                }
                _ => return,
            },
            _ => return,
        };
        let substs = cx.typeck_results().node_substs(expr.hir_id);
        if substs.needs_subst() {
            // We can't resolve on types that require monomorphization, so we don't handle them if
            // we need to perfom substitution.
            return;
        }
        let param_env = cx.tcx.param_env(trait_id);
        // Resolve the trait method instance.
        let i = match ty::Instance::resolve(cx.tcx, param_env, did, substs) {
            Ok(Some(i)) => i,
            _ => return,
        };
        // (Re)check that it implements the noop diagnostic.
        for (s, peel_ref) in [(sym::noop_method_clone, false)].iter() {
            if cx.tcx.is_diagnostic_item(*s, i.def_id()) {
                let method = &call.ident.name;
                let receiver = &elements[0];
                let receiver_ty = cx.typeck_results().expr_ty(receiver);
                let receiver_ty = match receiver_ty.kind() {
                    // Remove one borrow from the receiver if appropriate to positively verify that
                    // the receiver `&self` type and the return type are the same, depending on the
                    // involved trait being checked.
                    ty::Ref(_, ty, _) if *peel_ref => ty,
                    // When it comes to `Clone` we need to check the `receiver_ty` directly.
                    // FIXME: we must come up with a better strategy for this.
                    _ => receiver_ty,
                };
                let expr_ty = cx.typeck_results().expr_ty_adjusted(expr);
                if receiver_ty != expr_ty {
                    // This lint will only trigger if the receiver type and resulting expression \
                    // type are the same, implying that the method call is unnecessary.
                    return;
                }
                let expr_span = expr.span;
                let note = format!(
                    "the type `{:?}` which `{}` is being called on is the same as \
                     the type returned from `{}`, so the method call does not do \
                     anything and can be removed",
                    receiver_ty, method, method,
                );

                let span = expr_span.with_lo(receiver.span.hi());
                cx.struct_span_lint(NOOP_METHOD_CALL, span, |lint| {
                    let method = &call.ident.name;
                    let message = format!(
                        "call to `.{}()` on a reference in this situation does nothing",
                        &method,
                    );
                    lint.build(&message)
                        .span_label(span, "unnecessary method call")
                        .note(&note)
                        .emit()
                });
            }
        }
    }
}
