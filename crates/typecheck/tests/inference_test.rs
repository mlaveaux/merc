use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;
use merc_typecheck::InferenceError;
use merc_typecheck::WellTypedError;

/// Type checks `text`, asserting it is accepted.
#[track_caller]
fn check_ok(text: &str) {
    let spec = UntypedDataSpecification::parse(text).expect("the specification should parse");
    if let Err(err) = DataSpecification::from_untyped(spec) {
        panic!("expected the specification to type check, got {err}:\n{text}");
    }
}

/// Type checks `text`, returning the error for the caller to match on the
/// specific variant (never the message text, which may change).
#[track_caller]
fn check_err(text: &str) -> WellTypedError {
    let spec = UntypedDataSpecification::parse(text).expect("the specification should parse");
    match DataSpecification::from_untyped(spec) {
        Err(err) => err,
        Ok(_) => panic!("expected the specification to be rejected:\n{text}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_ambiguous_function_picks_unique_zero_arity() {
    // `f` bare can only be the zero-arity overload; the others need arguments.
    check_ok(
        "sort U; S; T;
         map f: Pos;
             f: Pos # Nat -> U;
             f: Pos # Pos -> S;
             f: Nat # Pos -> T;
             result: Pos;
         eqn result = f;",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_ambiguous_function_application_picks_by_arg_sorts() {
    // With x: Pos, y: Nat, only one of the four overloads structurally
    // unifies with each argument pattern (Nat cannot downcast to Pos), so
    // each application has a unique resolution despite the shared name `f`.
    check_ok(
        "sort U; S; T;
         map f: Pos;
             f: Pos # Nat -> U;
             f: Pos # Pos -> S;
             f: Nat # Pos -> T;
             result: Pos -> S;
         var x: Pos; y: Nat;
         eqn result(x) = f(x, x);",
    );
    check_ok(
        "sort U; S; T;
         map f: Pos;
             f: Pos # Nat -> U;
             f: Pos # Pos -> S;
             f: Nat # Pos -> T;
             result: Pos # Nat -> U;
         var x: Pos; y: Nat;
         eqn result(x, y) = f(x, y);",
    );
    check_ok(
        "sort U; S; T;
         map f: Pos;
             f: Pos # Nat -> U;
             f: Pos # Pos -> S;
             f: Nat # Pos -> T;
             result: Nat # Pos -> T;
         var x: Pos; y: Nat;
         eqn result(y, x) = f(y, x);",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_three_way_arity_overload_nested_application() {
    // `f` is overloaded 0/1/2-ary; a 3-way *arity* disjunction, distinct from
    // the 4-way *sort* disjunction above.
    check_ok(
        "map f: Nat -> Bool;
             f: Nat # Nat -> Bool;
             f: Nat;
             result: Nat;
         eqn result = f;",
    );
    check_ok(
        "map f: Nat -> Bool;
             f: Nat # Nat -> Bool;
             f: Nat;
             result: Bool;
         eqn result = f(f);",
    );
    check_ok(
        "map f: Nat -> Bool;
             f: Nat # Nat -> Bool;
             f: Nat;
             result: Bool;
         eqn result = f(f, f);",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_self_application_through_constant_and_function_overload() {
    // `f` overloaded as a constant `S` and as `S -> T`; applying the constant
    // overload to itself resolves to `T`.
    check_ok(
        "sort S; T;
         cons f: S;
              f: S -> T;
         map result: T;
         eqn result = f(f);",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_upcast_pos_plus_nat_via_variables() {
    // `+` and `==` over declared *variables* rather than a literal on one
    // side, as the existing literal-focused tests use. `Pos # Nat -> Pos` is
    // a direct Appendix-B overload here, no upcast needed.
    check_ok(
        "map result: Pos # Nat -> Pos;
         var x: Pos; y: Nat;
         eqn result(x, y) = x + y;",
    );
    check_ok(
        "map result: Pos # Nat -> Bool;
         var x: Pos; y: Nat;
         eqn result(x, y) = (x == y);",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_repeated_arithmetic_stays_tractable() {
    // This test guards that tractability for many repeated `2*i+k`-shaped
    // sub-expressions.
    check_ok(
        "map f: Nat -> Bool;
         var i: Nat;
         eqn f(i) = if(2*i+1==2*i+2,
                       if(2*i+3==2*i+4,
                          if(2*i+5==2*i+6,
                             if(2*i+7==2*i+8,
                                if(2*i+9==2*i+10,
                                   if(2*i+11==2*i+12,
                                      if(2*i+13==2*i+14,
                                         if(2*i+15==2*i+16,
                                            2*i+17==2*i+18,
                                            false),
                                         false),
                                      false),
                                   false),
                                false),
                             false),
                          false),
                       false);",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_list_literal_mixed_nat_pos_joins_to_nat() {
    check_ok("map l: List(Nat); eqn l = [0, 1, 2];");
    check_ok("map l: List(Nat); eqn l = [1, 0, 2];");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_list_concat_variable_upcast() {
    // A declared `List(Nat)`/`List(Pos)` variable concatenated with a
    // literal list stays at the variable's sort.
    check_ok("map r: List(Nat) -> List(Nat); var l: List(Nat); eqn r(l) = l ++ [1, 2];");
    check_ok("map r: List(Pos) -> List(Pos); var l: List(Pos); eqn r(l) = l ++ [1, 2];");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_list_concat_asymmetric_upcast() {
    // `[0] ++ l` succeeds when `l: List(Nat)` (the literal upcasts), but not
    // when `l: List(Pos)` (the literal `0` cannot downcast).
    check_ok("map r: List(Nat) -> List(Nat); var l: List(Nat); eqn r(l) = [0] ++ l;");
    let err = check_err("map r: List(Pos) -> List(Pos); var l: List(Pos); eqn r(l) = [0] ++ l;");
    assert!(
        matches!(err, WellTypedError::Inference(InferenceError::NoTyping { .. })),
        "{err}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_list_mismatched_variable_sorts_rejected() {
    // `List` has no sub-sort relation between element sorts (unlike
    // `FSet(S) <= Set(S)`), so `List(Pos)` and `List(Nat)` are simply
    // incomparable, both under `++` and `==`.
    let err =
        check_err("map r: List(Pos) # List(Nat) -> List(Nat); var x: List(Pos); y: List(Nat); eqn r(x, y) = x ++ y;");
    assert!(
        matches!(err, WellTypedError::Inference(InferenceError::NoTyping { .. })),
        "{err}"
    );

    let err =
        check_err("map b: List(Pos) # List(Nat) -> Bool; var x: List(Pos); y: List(Nat); eqn b(x, y) = (x == y);");
    assert!(
        matches!(err, WellTypedError::Inference(InferenceError::NoTyping { .. })),
        "{err}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_fbag_literal_widens_to_bag_at_use() {
    // The `Bag` analogue of the already-tested `FSet <= Set` widening; `Bag`
    // members are (value, multiplicity) pairs, a different code path.
    check_ok("map b: Bag(Nat); eqn b = {0: 2, 1: 3};");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_exp_operator_sort() {
    // `exp: Pos # Nat -> Pos` needs one upcast (the exponent); `exp: Nat #
    // Nat -> Nat` would need two, so the ranked solver prefers the former.
    check_ok("map p: Pos; eqn p = exp(2, 3);");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_mod_upcasts_positive_dividend_to_nat() {
    // `mod: Nat # Pos -> Nat` is the only overload; a `Pos` dividend upcasts.
    check_ok("map n: Pos -> Nat; var x: Pos; eqn n(x) = x mod 2;");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_div_over_int_stays_int() {
    check_ok("map r: Int -> Int; var x: Int; eqn r(x) = x div 2;");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_int2pos_conversion_family() {
    // Exercises every downcast conversion name at once, a regression net for
    // the basic-sort system signature.
    check_ok(
        "map fpos: Pos -> Bool;
             fnat: Nat -> Bool;
             fint: Int -> Bool;
             result: Bool;
         eqn result = fpos(Nat2Pos(0)) && fpos(Int2Pos(-1)) && fpos(Real2Pos(1 / 2)) &&
                      fnat(Int2Nat(-1)) && fnat(Real2Nat(1 / 2)) &&
                      fint(Real2Int(1 / 2));",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_avoidance_of_possible_types_regression() {
    // A stale "PossibleTypes([Nat,Int,Real])" sort for `#` used to leak past
    // the `==` scheme.
    check_ok("map result: Bool; eqn result = (#[0, 1] == -1);");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_eqn_set_where() {
    // A `whr` inside a set comprehension. Since the binders commit this is
    // genuinely inferred (`Set(Bool)` through the `if` scheme with the `{}`
    // widened `FSet <= Set`), no longer skipped.
    check_ok(
        "map f_dot: Set(Bool);
         eqn f_dot = if(true, {}, { o: Bool | true whr z = true end });",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_function_update_chain_without_lambda() {
    check_ok(
        "map f: Bool -> Bool; g: Bool -> Bool;
         eqn g = f[true -> false][false -> true];",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_function_updates() {
    // Function updates on `lambda` bases, incl. a chained update and a
    // mismatched point sort.
    check_ok("map f: Bool -> Bool; eqn f = (lambda x: Bool. x)[true -> false];");
    check_ok("map f: Bool -> Bool; eqn f = (lambda x: Bool. x)[true -> false][false -> true];");
    check_ok("map f: Nat -> Bool; eqn f = (lambda n: Nat. n mod 2 == 0)[0 -> false];");
    let err = check_err("map f: Bool -> Bool; eqn f = (lambda x: Bool. x)[0 -> false];");
    assert!(
        matches!(err, WellTypedError::Inference(InferenceError::NoTyping { .. })),
        "{err}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_matching_multi_param_distinct_sorts() {
    // Two-parameter declaration-sort matching over a real product domain,
    // distinct from the single-parameter cases already tested.
    check_ok(
        "map f: Pos # Nat -> Bool;
         var x: Pos; y: Nat;
         eqn f(x, y) = true;",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_matching_repeated_variable_non_strict() {
    // The same variable filling two parameter positions of different
    // declared sorts; `x` upcasts into the `Nat` slot.
    check_ok(
        "map f: Pos # Nat -> Bool;
         var x: Pos;
         eqn f(x, x) = true;",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_aliased_list_of_list_equality() {
    // Alias normalization reaching through two nested container levels
    // feeding the `==` scheme.
    check_ok(
        "sort B; A = List(List(B)); C = List(B);
         map result: A # List(C) -> Bool;
         var f: A; g: List(C);
         eqn result(f, g) = (f == g);",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_ambiguous_projection_function_resolves() {
    // `pi_1` is overloaded across two struct alternatives (`T1(pi_1: T)`,
    // `T2(pi_1: S)`), and `IS_T1(p)` in the same conjunct disambiguates which
    // one applies.
    check_ok(
        "sort S;
             T = struct T0 | T1(pi_1: T)?IS_T1 | T2(pi_1: S)?IS_T2;
         map R: T -> Bool;
             result: T -> Bool;
         var p: T;
         eqn result(p) = R(pi_1(p)) && IS_T1(p);",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_boolean_operator_basics() {
    check_ok("map b: Bool; eqn b = true;");
    check_ok("map b: Bool; eqn b = if(true, true, false);");
    check_ok("map b: Bool; eqn b = !true;");
    check_ok("map b: Bool; eqn b = true && false;");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_number_literal_operator_sorts() {
    // `+` takes its heterogeneous overloads: a `Pos` on either side yields
    // `Pos`.
    check_ok("map n: Nat; eqn n = 0;");
    check_ok("map i: Int; eqn i = -1;");
    check_ok("map p: Pos; eqn p = 0 + 1;");
    check_ok("map p: Pos; eqn p = 1 + 0;");
    check_ok("map n: Nat; eqn n = 0 + 0;");
    check_ok("map p: Pos; eqn p = 1 + 1;");
    check_ok("map p: Pos; eqn p = 1 * 2 + 3;");
}

// List literals and empty-list typing

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_empty_list_takes_element_sort_from_use() {
    // Merc's constraint solver can use the context to resolve the element sort.
    check_ok("map l: List(Bool); eqn l = [];");
    check_ok("map l: List(Bool); eqn l = [] ++ [];");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_empty_list_membership() {
    // The member's sort determines the empty list's element sort through the
    // polymorphic `in` template.
    check_ok("map b: Bool; eqn b = true in [];");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_list_literal_sorts() {
    check_ok("map l: List(Bool); eqn l = [true, false];");
    check_ok("map l: List(Nat); eqn l = [0];");
    check_ok("map l: List(Pos); eqn l = [1, 2];");
    check_ok("map l: List(Nat); eqn l = [0] ++ [1, 2];");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_head_of_list_literal() {
    check_ok("map n: Nat; eqn n = head([0]);");
    check_ok("map n: Nat; eqn n = head([0, 1]);");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_emptyset_complement() {
    // `!` on sets comes from the polymorphic template.
    check_ok("map s: Set(Bool); eqn s = !{};");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_set_complement_subset_with_context() {
    // The faithful `!{} <= {}` (either side empty) is the known-gap anchor
    // test_emptyset_complement_subset below; with the element sort supplied by a
    // variable, complement-under-subset itself types fine.
    check_ok("map b: Set(Nat) -> Bool; var s: Set(Nat); eqn b(s) = !{} <= s;");
    check_ok("map b: Set(Nat) -> Bool; var s: Set(Nat); eqn b(s) = s <= !{};");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_emptybag_complement_rejected() {
    // Bags have no complement.
    let err = check_err("map b: Bag(Bool); eqn b = !{:};");
    assert!(
        matches!(err, WellTypedError::Inference(InferenceError::NoTyping { .. })),
        "{err}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_set_literal_sorts() {
    // A negative member joins the elements to `Int`.
    check_ok("map s: FSet(Bool); eqn s = {true, false};");
    check_ok("map s: FSet(Int); eqn s = {1, 2, -7};");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_set_comprehension_with_mod_body() {
    check_ok("map s: Set(Nat); eqn s = { x: Nat | x mod 2 == 0 };");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_fset_count_and_pick() {
    // `#` and `pick` through the polymorphic container templates.
    check_ok("map n: Nat; eqn n = #{true, false};");
    check_ok("map b: Bool; eqn b = pick({true, false});");
    check_ok("map n: Nat; eqn n = pick({0, 1});");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_bag_literal_sorts() {
    check_ok("map f: FBag(Bool); eqn f = {true: 1, false: 2};");
    check_ok("map f: FBag(Int); eqn f = {1: 1, 2: 2, -8: 8};");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_fbag_count_and_pick() {
    check_ok("map n: Nat; eqn n = #{1: 1, 2: 2, -8: 8};");
    check_ok("map i: Int; eqn i = pick({1: 1, 2: 2, -8: 8});");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_bag_comprehension_with_lambda_body() {
    // A lambda applied inside the multiplicity body.
    check_ok("map b: Bag(Nat); eqn b = { x: Nat | (lambda y: Nat. y * y)(x) };");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_bag_comprehension_body_sorts() {
    // A `Pos` body (`n + 1`) and a `Nat` literal body read as bags; a `Real`
    // body is rejected.
    check_ok("map b: Bag(Pos); eqn b = { n: Pos | n + 1 };");
    check_ok("map b: Bag(Pos); eqn b = { n: Pos | 0 };");
    let err = check_err("map b: Bag(Pos); eqn b = { n: Pos | 2 / 3 };");
    assert!(
        matches!(err, WellTypedError::Inference(InferenceError::NoTyping { .. })),
        "{err}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_lambda_term_with_wrong_number_of_arguments() {
    // A unary lambda applied to two arguments.
    let err = check_err("map b: Bool; eqn b = (lambda x: Nat. x)(1, 2) > 0;");
    assert!(
        matches!(err, WellTypedError::Inference(InferenceError::NotAFunction { .. })),
        "{err}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_lambda_aliasing() {
    // The inner `f` shadows the outer for the body, so `f(f)` must apply the
    // function to itself, which cannot unify.
    let err = check_err(
        "map g: Nat -> (Nat -> Bool) -> Bool;
         eqn g = lambda f: Nat. lambda f: Nat -> Bool. f(f);",
    );
    assert!(
        matches!(err, WellTypedError::Inference(InferenceError::NoTyping { .. })),
        "{err}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_lambda_variable_aliasing() {
    // The lambda's `x: S` shadows the declared `x: S -> T`, so `x(x)`
    // applies a non-function.
    let text = "sort S; T; map h: S -> Bool; var x: S -> T; eqn h = lambda x: S. x(x);";
    let err = check_err(text);
    match &err {
        WellTypedError::Inference(InferenceError::NotAFunction { expr, span }) => {
            assert_eq!(expr, "x");
            // The span points at the applied `x` (bound by the lambda, sort
            // `S`), not the earlier `var x: S -> T` declaration.
            let callee = text.find("x(x)").expect("the application is present");
            assert_eq!(span.start, callee);
            assert_eq!(&text[span.start..span.end], "x");
        }
        other => panic!("expected NotAFunction, got {other}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_forall_nat_vs_int_body() {
    // The body compares a `Nat` variable with a negative literal, joining at
    // `Int`.
    check_ok("map b: Bool; eqn b = forall n: Nat. n > -1;");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_exists_simple() {
    check_ok("map b: Bool; eqn b = exists n: Nat. n > 481;");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_binders_over_anonymous_structs_accepted() {
    // Anonymous `struct` binder sorts are hoisted and fully inferred (not
    // skipped), so these type check like any other equation.
    check_ok("map b: (struct t) # (struct t) -> Bool; eqn b = lambda x,y: struct t. x == y;");
    check_ok("map b: Bool; eqn b = forall x,y: struct t. x == y;");
    check_ok("map b: Bool; eqn b = exists x,y: struct t. x == y;");
    check_ok("map f: (struct t) -> Bool; g: (struct t) -> Bool; eqn g = lambda x: struct t. f(x);");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_product_binder_sort_is_rejected() {
    // A bare product (`Nat # Bool`) is not a valid variable sort, so a binder
    // over one is rejected rather than left untyped — the former silent skip
    // let an ill-typed body under such a binder slip through unchecked.
    for text in [
        "map b: Bool; eqn b = forall x: Nat # Bool. true;",
        "map b: Bool; eqn b = exists x: Nat # Bool. true;",
        "map b: Bool; eqn b = forall x: Nat # Bool. 1 + true;",
        "map s: Set(Nat); eqn s = { x: Nat # Nat | true };",
    ] {
        let err = check_err(text);
        assert!(
            matches!(err, WellTypedError::Inference(InferenceError::InvalidBinderSort { .. })),
            "{err} for {text}"
        );
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_anonymous_struct_variable_sorts() {
    // Anonymous structs in a `var` block are hoisted, and structurally
    // identical ones share one hoisted declaration, so equal binder sorts
    // compare while a recogniser makes the sorts distinct.
    check_ok(
        "map b: (struct t?is_t) # (struct t?is_t) -> Bool; var x: struct t?is_t; y: struct t?is_t; \
         eqn b(x, y) = (x == y);",
    );
    // With non-decl hoisting, `struct t` and `struct t?is_t` each hoist to
    // abstract sorts (no constructors), so no duplicate-constant collision
    // occurs at the signature stage. Instead inference rejects `x == y`
    // because `x: @struct0` and `y: @struct1` are distinct nominal sorts with
    // no common supersort.
    let err = check_err(
        "map b: (struct t) # (struct t?is_t) -> Bool; var x: struct t; y: struct t?is_t; \
         eqn b(x, y) = (x == y);",
    );
    assert!(
        matches!(err, WellTypedError::Inference(InferenceError::NoTyping { .. })),
        "{err}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_where_basic() {
    check_ok("map p: Pos; eqn p = x + y whr x = 3, y = 10 end;");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_where_bindings_use_outer_scope_only() {
    // A sibling binding's name is not in scope for a right-hand side, so
    // without an outer declaration the reference is undeclared.
    for spec in [
        "map p: Pos; eqn p = x + y whr x = 3, y = x + 10 end;",
        "map p: Pos; eqn p = x + y whr x = 3, y = x + y + 10 end;",
        "map p: Pos; eqn p = x + y whr x = y + 10, y = 3 end;",
        "map p: Pos; eqn p = x + y whr x = y + 10, y = x + 3 end;",
    ] {
        let err = check_err(spec);
        assert!(
            matches!(err, WellTypedError::Inference(InferenceError::UndeclaredName { .. })),
            "{err}"
        );
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_where_bindings_resolve_against_declared_variables() {
    // With outer declarations, every right-hand side types against the
    // declared variables (not the sibling bindings).
    check_ok("map p: Pos # Nat -> Pos; var x: Pos; y: Nat; eqn p(x, y) = x + y whr x = 3, y = 0 end;");
    check_ok("map p: Pos # Pos -> Pos; var x: Pos; y: Pos; eqn p(x, y) = x + y whr x = 3, y = x + 10 end;");
    check_ok("map p: Pos # Pos -> Pos; var x: Pos; y: Pos; eqn p(x, y) = x + y whr x = 3, y = x + y + 10 end;");
    check_ok("map p: Pos # Nat -> Pos; var x: Pos; y: Nat; eqn p(x, y) = x + y whr x = y + 10, y = 0 end;");
    check_ok("map p: Pos # Pos -> Pos; var x: Pos; y: Pos; eqn p(x, y) = x + y whr x = y + 10, y = x + 3 end;");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_where_mix_nat_list() {
    check_ok("map l: Nat # Nat -> List(Nat); var x: Nat; z: Nat; eqn l(x, z) = x1 ++ y whr x1 = [0, z], y = [x] end;");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_where_mix_nat_pos_list_types_globally() {
    // Type both bindings at List(Nat) — the `[x]` element upcasts Pos <= Nat —
    // which is a coherent assignment, so the equation is accepted.
    check_ok("map l: Pos # Nat -> List(Nat); var x: Pos; y: Nat; eqn l(x, y) = x ++ y whr x = [0, y], y = [x] end;");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_bare_overloaded_name_is_ambiguous() {
    // A bare `f` with several overloads has no unique sort.
    for spec in [
        "map f: Nat -> Bool; f: Nat # Nat -> Bool; b: Bool; eqn b = (f == f);",
        "map f: Nat -> Nat -> Bool; f: Nat -> Bool; b: Bool; eqn b = (f == f);",
        "map f: Pos -> Nat; f: Nat -> Pos; b: Bool; eqn b = (f == f);",
    ] {
        let err = check_err(spec);
        assert!(
            matches!(
                err,
                WellTypedError::Inference(InferenceError::AmbiguousExpression { .. })
            ),
            "{err}"
        );
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_zero_arity_overload_resolution() {
    // A bare `f` with exactly one zero-arity overload resolves regardless of
    // declaration order, and `f(f)` threads the constant through the unary
    // overload.
    check_ok("map f: Nat -> Bool; f: Nat; r: Nat; eqn r = f;");
    check_ok("map f: Nat; f: Nat -> Bool; r: Nat; eqn r = f;");
    check_ok("map f: Nat -> Bool; f: Nat; b: Bool; eqn b = f(f);");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_arity_overloads_through_lambda_application() {
    // The 0/1/2-ary `f` family threaded through an applied lambda.
    check_ok(
        "map f: Nat -> Bool; f: Nat # Nat -> Bool; f: Nat; b: Bool;
         eqn b = f((lambda x: Bool. f)(f(f, f)));",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_same_arity_overloads_resolved_by_argument() {
    // `f: Pos -> Nat` vs `f: Nat -> Pos`: a `Nat` argument cannot downcast
    // to `Pos`, and for a `Pos` argument the ranked solver prefers the exact
    // application over the upcast one.
    check_ok("map f: Pos -> Nat; f: Nat -> Pos; r: Pos; eqn r = f(0);");
    check_ok("map f: Pos -> Nat; f: Nat -> Pos; r: Nat; eqn r = f(1);");
    check_ok("map f: Pos -> Nat; f: Nat -> Pos; r: Nat -> Pos; var x: Nat; eqn r(x) = f(x);");
    check_ok("map f: Pos -> Nat; f: Nat -> Pos; r: Pos -> Nat; var x: Pos; eqn r(x) = f(x);");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_function_application_argument_upcasts() {
    // `f: Nat -> Bool` accepts `Pos` arguments (upcast) and rejects `Int`
    // ones (no downcast), for literals and variables alike.
    check_ok("map f: Nat -> Bool; g: Nat -> Bool; eqn g = f;");
    check_ok("map f: Nat -> Bool; b: Bool; eqn b = f(1);");
    check_ok("map f: Nat -> Bool; b: Bool; eqn b = f(0);");
    check_ok("map f: Nat -> Bool; b: Pos -> Bool; var x: Pos; eqn b(x) = f(x);");
    check_ok("map f: Nat -> Bool; b: Nat -> Bool; var x: Nat; eqn b(x) = f(x);");
    for spec in [
        "map f: Nat -> Bool; b: Bool; eqn b = f(-1);",
        "map f: Nat -> Bool; b: Int -> Bool; var x: Int; eqn b(x) = f(x);",
    ] {
        let err = check_err(spec);
        assert!(
            matches!(err, WellTypedError::Inference(InferenceError::NoTyping { .. })),
            "{err}"
        );
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_struct_constructor_applications() {
    // A struct constructor `c: Nat -> S` behaves like any mapping under
    // application and upcasting.
    check_ok("sort S = struct c(Nat); map g: Nat -> S; eqn g = c;");
    check_ok("sort S = struct c(Nat); map r: S; eqn r = c(1);");
    check_ok("sort S = struct c(Nat); map r: S; eqn r = c(0);");
    check_ok("sort S = struct c(Nat); map r: Pos -> S; var x: Pos; eqn r(x) = c(x);");
    check_ok("sort S = struct c(Nat); map r: Nat -> S; var x: Nat; eqn r(x) = c(x);");
    for spec in [
        "sort S = struct c(Nat); map r: S; eqn r = c(-1);",
        "sort S = struct c(Nat); map r: Int -> S; var x: Int; eqn r(x) = c(x);",
    ] {
        let err = check_err(spec);
        assert!(
            matches!(err, WellTypedError::Inference(InferenceError::NoTyping { .. })),
            "{err}"
        );
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_data_expressions_struct() {
    // Constructor application through a nested anonymous struct declaration.
    check_ok("sort S = struct t(struct e(Nat)); map b: S -> Bool; var x: S; eqn b(x) = (x == t(e(3)));");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_proper_use_of_int2pos() {
    // The whole conversion family is tested in test_int2pos_conversion_family
    // above.
    check_ok("map f: Pos -> Bool; b: Bool; eqn b = f(Int2Pos(-1));");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_ambiguous_function_application_recursive() {
    // Resolves with f: Pos -> Int (exact into g) over f: Pos -> Nat.
    check_ok("map g: Int -> Bool; f: Pos -> Nat; f: Pos -> Int; b: Pos -> Bool; var x: Pos; eqn b(x) = g(f(x));");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_ambiguous_function_application_recursive2() {
    // The added g: Int -> Int is filtered out by the equation's Bool
    // left-hand side.
    check_ok(
        "map g: Int -> Bool; f: Pos -> Nat; f: Pos -> Int; g: Int -> Int; b: Pos -> Bool; var x: Pos;
         eqn b(x) = g(f(x));",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_ambiguous_function_application_recursive3() {
    // Resolves with f: Pos -> Nat (argument exact, result upcast) over
    // f: Int -> Int.
    check_ok(
        "map g: Int -> Bool; f: Pos -> Nat; f,g: Int -> Int; b: Pos -> Bool; var x: Pos;
         eqn b(x) = g(f(x));",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_ambiguous_function_application_recursive4() {
    // g: Nat -> Int is filtered by the Bool left-hand side; f resolves as in
    // the first case.
    check_ok(
        "map g: Int -> Bool; f: Pos -> Nat; f: Pos -> Int; g: Nat -> Int; b: Pos -> Bool; var x: Pos;
         eqn b(x) = g(f(x));",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_improvement_ranked_overload_through_list_literal() {
    // Same limitation as test_ambiguous_function_application_recursive, but the
    // disambiguating context is a container literal rather than a function
    // application.
    check_ok(
        "map h: List(Nat) -> Bool; f: Pos -> Nat; f: Pos -> Pos; b: Pos -> Bool; var x: Pos; eqn b(x) = h([f(x)]);",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_improvement_ranked_overload_two_level_nesting() {
    // Two nested overloaded calls. `bot` is unique, but `mid` reaches the `Int`
    // domain of `top` exactly (`mid: Nat -> Int`) or by upcast (`mid: Nat ->
    // Nat`, `Nat <= Int`).
    check_ok(
        "map top: Int -> Bool; mid: Nat -> Int; mid: Nat -> Nat; bot: Pos -> Nat; b: Pos -> Bool;
         var x: Pos; eqn b(x) = top(mid(bot(x)));",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_improvement_where_global_int_list() {
    // Like test_where_mix_nat_pos_list_types_globally, but the result is
    // `List(Int)` and a negative literal forces `Int`.
    check_ok("map l: Pos # Nat -> List(Int); var x: Pos; y: Nat; eqn l(x, y) = x ++ y whr x = [-1, y], y = [x] end;");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_improvement_ambiguous_projection_disambiguated_by_use() {
    // `val` is overloaded across two struct alternatives (`A(val: T)`,
    // `B(val: S)`), and `use: S -> Bool` in the same conjunct forces the
    // `T -> S` overload, consistent with `is_B(p)`.
    check_ok(
        "sort S; T = struct A(val: T)?is_A | B(val: S)?is_B | T0;
         map use: S -> Bool; result: T -> Bool;
         var p: T; eqn result(p) = use(val(p)) && is_B(p);",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_improvement_ranked_overload_through_equation_result() {
    // Tthe equation's expected sort (`Nat`, from the `result` mapping)
    // propagates into the overloaded inner `f`, selecting `f: Pos -> Nat` over
    // `f: Pos -> Pos`.
    check_ok(
        "map wrap: Nat -> Nat; f: Pos -> Nat; f: Pos -> Pos; result: Nat;
         var x: Pos; eqn result = wrap(f(x));",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_matching_ambiguous() {
    // Upstream expected accept but keeps the case disabled over
    // pretty-printer reordering (not a typechecking issue): the exact
    // `Pos # Nat` overload outranks `Nat # Nat` for `f(x, y)`, and `f(y, y)`
    // only fits `Nat # Nat`.
    check_ok(
        "map f: Pos # Nat -> Bool; f: Nat # Nat -> Bool;
         var x: Pos; y: Nat; eqn f(x, y) = false;
         var x: Pos; y: Nat; eqn f(y, y) = true;",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_matching_ambiguous_rhs() {
    // A constant `f: Int` applied to an argument on an equation left-hand
    // side; the original's second equation (`f(x) = 3;`) is dropped since
    // the rejection already fires on the first.
    let err = check_err("map f: Int; var x: Pos; eqn f(x) = -5;");
    assert!(
        matches!(err, WellTypedError::Inference(InferenceError::NotAFunction { .. })),
        "{err}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_ambiguous_function_application4_with_expected_sort() {
    // Upstream (disabled) expected `f(x, x)` under an *unknown* expected
    // sort to resolve to `Nat # Nat -> S`, i.e. expand-all-arguments
    // semantics. merc's equation entry always has an expected sort, which
    // determines the overload either way; the unknown-expected reading
    // (lexicographic-nearest would pick `U`, mCRL2 intended `S`) is a
    // known permissive divergence.
    check_ok(
        "sort S; T; U; map f: Pos; f: Pos # Nat -> U; f: Nat # Nat -> S; f: Nat # Pos -> T; result: U;
         var x: Pos; y: Nat; eqn result = f(x, x);",
    );
    check_ok(
        "sort S; T; U; map f: Pos; f: Pos # Nat -> U; f: Nat # Nat -> S; f: Nat # Pos -> T; result: S;
         var x: Pos; y: Nat; eqn result = f(x, x);",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
// Free element sort is defaulted to Bool, matching mCRL2's acceptance.
fn test_count_of_empty_list_is_nat() {
    check_ok("map n: Nat; eqn n = #[];");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
// Free element sort is defaulted to Bool, matching mCRL2's acceptance.
fn test_emptyset_complement_subset() {
    check_ok("map b: Bool; eqn b = !{} <= {};");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
// Free element sort is defaulted to Bool, matching mCRL2's acceptance.
fn test_emptyset_complement_subset_reverse() {
    check_ok("map b: Bool; eqn b = {} <= !{};");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_inline_struct_rejected() {
    check_err("map b: (struct t) -> Bool; eqn b = lambda x: struct t. x == t;");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_inline_struct_recogniser_rejected() {
    check_err("map b: (struct t?is_t) -> Bool; eqn b = lambda x: struct t?is_t. x == t;");
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
// `struct t?is_t` and `struct t` each hoist to abstract sorts (different
// non-decl sorts), so `x: @struct0` and `y: @struct1` have incompatible
// sorts and `x == y` has no valid typing.
fn test_inline_structs_compare_recogniser_rejected() {
    check_err(
        "map b: (struct t?is_t) # (struct t) -> Bool;
         eqn b = lambda x: struct t?is_t, y: struct t. x == y;",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
// Regression for a former blow-up: the `if` join of a finite-set-literal
// branch with a set-comprehension branch used to bind the result to `FSet`
// first and then re-explore the whole comprehension body fruitlessly.
// Computing the branch join as a single lattice least-upper-bound (`Join`)
// types it in one step. This reduced shape
// captures the pattern; the full spec is covered by the example corpus.
fn test_if_joins_set_literal_and_comprehension() {
    check_ok(
        "sort Transition = struct trans(src: Nat, tar: Nat);
         map T: Nat -> Set(Transition);
         var i: Nat;
         eqn T(i) = if(i == 0,
                       { trans(0, 0), trans(1, 1) },
                       { t: Transition | src(t) == 2 * i + 1 });",
    );
}
