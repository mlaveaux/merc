//! The specifications of mCRL2's linearisation tests (`libraries/lps/test/linearization_test*.cpp`),
//! with the outcome mCRL2 expects. A specification mCRL2 linearises must be linearised by merc too,
//! unless it uses a construct merc does not support (time and stochastic operators); one that mCRL2
//! rejects must be rejected. Linearisation never panics.

use test_case::test_case;

use merc_linearisation::LinearisationError;
use merc_linearisation::LinearisationOptions;
use merc_lps::read_lps;
use merc_lps::write_lps;
use merc_syntax::UntypedProcessSpecification;
use merc_typecheck::ProcessSpecification;

#[test_case(r###"act a;

proc X(i: Nat) = a.X(i);

init X(2);
"###, true ; "linearization_test1_case_1_0")]
#[test_case(r###"act a: Nat;

proc X(i,j: Nat) = a(i). X(i,j);

init X(0,1);
"###, true ; "linearization_test1_case_2_1")]
#[test_case(r###"act a;

proc X(i,j: Nat)   = (i == 5) -> a. X(i,j);

init X(0,1);
"###, true ; "linearization_test1_case_3_2")]
#[test_case(r###"act a;

proc X(i,j: Nat) = a@i.X(i,j);

init X(0,4);
"###, true ; "linearization_test1_case_4_3")]
#[test_case(r###"act a: Nat;
act b;

proc X(i,j,k: Nat) =  a(i).X(k,j,k) +
                         b.X(j,j,k);

init X(1,2,3);"###, true ; "linearization_test1_case_5_4")]
#[test_case(r###"act act1, act2, act3: Nat;

proc X(i: Nat)   = (i <  5) -> act1(i).X(i+1) +
                   (i == 5) -> act3(i).Y(i, i);
     Y(i,j: Nat) = act2(j).Y(i,j+1);

init X(0);
"###, true ; "linearization_test1_case_6_5")]
#[test_case(r###"act act1, act2, act3: Nat;

proc X(i,z,j: Nat)   = (i <  5) -> act1(i)@z.X(i+1,z, j) +
                       (i == 5) -> act3(i).X(i, j, 4);

init X(0,5, 1);
"###, true ; "linearization_test1_case_7_6")]
#[test_case(r###"act a;
init sum t:Nat. a@t;
"###, true ; "linearization_test1_case_8_7")]
#[test_case(r###"act c;
init sum t:List(struct a | b) . c;
"###, true ; "linearization_test1_case_9_8")]
#[test_case(r###"act a,b,c;proc X(v:Nat)=a.X(v=3)+Y(1,2);Y(v1:Nat, v2:Nat)=a.Y(v1=3)+b.X(5)+c.Y(v2=7);init X(3);"###, true ; "linearization_test1_assignment_case_1_9")]
#[test_case(r###"act a;proc X(v:Nat)=a.Y(w=true);Y(w:Bool)=a.X(v=0);init X(v=3);"###, true ; "linearization_test1_assignment_case_2_10")]
#[test_case(r###"act a;    b:Nat;proc X(v:Nat,w:List(Bool))=a.X(w=[])+                         (v>0) ->b(v).X(v=max(v,0));init X(v=3,w=[]);"###, true ; "linearization_test1_assignment_case_3_11")]
#[test_case(r###"act a;proc X(v:Pos,w:Nat)=sum w:Pos.a.X(v=w)+                    sum u:Pos.a.X(v=u);init X(3,4);"###, true ; "linearization_test1_assignment_case_4_12")]
#[test_case(r###"act a;proc X(v:Pos)=sum v:Pos.a@4.X();init X(3);"###, true ; "linearization_test1_assignment_case_5_13")]
#[test_case(r###"sort D = struct d1(Nat)?is_d1 | d2(arg2:Nat)?is_d2;
                                                   
init true->delta;                                  
"###, true ; "linearization_test1_text_14")]
#[test_case(r###"act a,b:Int;
proc P = sum y:Int . (y == 4) -> a(y)@y . b(y*2)@(y+1) . P;
init P;
"###, true ; "linearization_test1_no_free_variables_case_1_15")]
#[test_case(r###"act a;proc X=a.X;init X;"###, true ; "linearization_test1_various_case_2_16")]
#[test_case(r###"sort D     = struct d1 | d2;             Error = struct e;act r2: D # Bool;    s3: D # Bool;    s3: Error;    i;proc K = sum d:D,b:Bool. r2(d,b).(i.s3(d,b)+i.s3(e)).K;init K;"###, true ; "linearization_test1_various_case_3_17")]
#[test_case(r###"act a:Nat;proc X=sum n:Nat. (n==0)->a(n).X;init X;"###, true ; "linearization_test1_various_case_4_18")]
#[test_case(r###"act a,b,c;proc X=a.X;     Y=b.Y;init X||Y;"###, true ; "linearization_test1_various_case_5_19")]
#[test_case(r###"act a1,a2,b,c;proc X=a1.a2.X;     Y=b.Y;init comm({a1|b->c},X||Y);"###, true ; "linearization_test1_various_case_6_20")]
#[test_case(r###"proc X=tau.X;init X;"###, true ; "linearization_test1_various_case_7_21")]
#[test_case(r###"act a,b;proc X= (a|b).X;init X;"###, true ; "linearization_test1_various_case_8_22")]
#[test_case(r###"act a;init allow({a},a.a.delta);"###, true ; "linearization_test1_various_case_9_23")]
#[test_case(r###"act a,b,c;init comm({a|b->c},(a|b).delta);"###, true ; "linearization_test1_various_case_10_24")]
#[test_case(r###"act a,b,c:Nat;map n:Nat;init comm({a|b->c},(a(3)|b(n)));"###, true ; "linearization_test1_various_case_11_25")]
#[test_case(r###"act c2:Nat#Nat;init allow({c2},c2(3,5));"###, true ; "linearization_test1_various_case_12_26")]
#[test_case(r###"sort D = struct d1 | d2;act r1,s4: D;proc S(b:Bool)     = sum d:D. r1(d).S(true);init S(false);"###, true ; "linearization_test1_various_case_13_27")]
#[test_case(r###"act r1: Bool;proc S(d:Bool) = sum d:Bool. r1(d).S(true);init S(false);"###, true ; "linearization_test1_various_case_14_28")]
#[test_case(r###"act a;init (a+a.a+a.a.a+a.a.a.a).delta;"###, true ; "linearization_test1_various_case_15_29")]
#[test_case(r###"act s6,r6,c6, i;proc T = r6.T;     K = i.K;     L = s6.L;init comm({r6|s6->c6},T || K || L);"###, true ; "linearization_test1_various_case_16_30")]
#[test_case(r###"act s3,r3,c3,s6;proc R = r3.R;     K = s3.K;     L = s6.L;init comm({r3|s3->c3}, K || L || R);"###, true ; "linearization_test1_various_case_17_31")]
#[test_case(r###"act a,b,c,d,e;init comm({c|d->b},(a|b|c|d|e).delta);"###, true ; "linearization_test1_various_case_18_32")]
#[test_case(r###"act a,b,c,d,e;init comm({e|d->b},(a|b|c|d|e).delta);"###, true ; "linearization_test1_various_case_19_33")]
#[test_case(r###"act a:Nat;proc X(n:Nat)=  sum n:Nat.(n>25) -> a(n).X(n)+  sum n:Nat.(n>25) -> a(n).X(n)+  (n>25) -> a(n).X(n);init X(1);"###, true ; "linearization_test1_various_case_20_34")]
#[test_case(r###"act a,b:Pos;proc X(m:Pos)= sum n:Nat. (n<1) -> a(1)|b(1)@1.X(1)+               sum n:Nat. (n<2) -> a(2)|b(2)@2.X(2)+               sum n:Nat. (n<3) -> a(3)|b(3)@3.X(3)+               sum n:Nat. (n<4) -> a(4)|b(4)@4.X(4)+               sum n:Nat. (n<5) -> a(5)|b(5)@5.X(5);init X(1);"###, true ; "linearization_test1_various_case_21_35")]
#[test_case(r###"% This test is expected to fail with a proper error message.
act a;
proc P = (a || a) . P;
init P;
"###, false ; "linearization_test1_various_case_22_36")]
#[test_case(r###"act a,b;init a@1.b@2.delta||tau.tau;"###, true ; "linearization_test1_various_case_23_37")]
#[test_case(r###"act  a: Pos;glob x: Pos;proc P = a(x).P;init P;"###, true ; "linearization_test1_various_case_24_38")]
#[test_case(r###"act a:Pos#Pos#Pos;    b;proc Q(y:Pos)=P(y,1,1)||delta;     P(x1,x2,x3:Pos)=a(x1,x2,x3).P(x1,x2,x3+1);init Q(2);"###, true ; "linearization_test1_various_case_25_39")]
#[test_case(r###"act  r,s1,s2:Nat;
proc P=sum m:Nat.r(m).((m==1)->s1(m).P+(m==2)->P+P);
init P;
"###, true ; "linearization_test1_various_case_26_40")]
#[test_case(r###"act  a: D;
sort D;

proc X1 = sum d: D. a(d) . X1;

init sum d1: D. a(d1) . X1;
"###, true ; "linearization_test1_report_1553_41")]
#[test_case(r###"map
  f : Nat # Nat -> Nat;
  f : Nat -> Nat;
  g : Nat # Nat -> Nat;
  g : Nat -> Nat;

eqn
  f = g;

init delta;
"###, false ; "linearization_test1_report_1576_42")]
#[test_case(r###"map select : (Nat -> Bool) # List(Nat) -> List(Nat);
var f : Nat -> Bool;
    x : Nat;
    xs : List(Nat);
eqn select(f,[]) = [];
    select(f,x|>xs) = if(f(x), x|>sxs, sxs) whr sxs = select(f, xs) end;
act a : Nat;
init sum n : Nat.
  (n in select(lambda x : Nat.x mod 2 == 0, [1, 2, 3])) -> a(n).delta;
"###, true ; "linearization_test1_inline_43")]
#[test_case(r###"init delta;"###, true ; "linearization_test1_inline_44")]
#[test_case(r###"act a:Pos;
proc P(id,n:Pos)=(id<n) -> a(n).P(id,n);
     Q(n:Pos)=P(1,n)||P(2,n)||P(3,n);
init Q(15);
"###, true ; "linearization_test2_various_case_27_45")]
#[test_case(r###"sort A=List(Nat->Nat);T=struct f(Nat->Nat);act b:A;proc P(a:A)=b(a).P([]);init P([lambda n:Nat.n]);"###, true ; "linearization_test2_various_case_28_46")]
#[test_case(r###"sort Data = struct x;Coloured = struct flow(data : Data) | noflowG | noflowR;act A, B : Coloured;proc Sync = ( (A(noflowG) | B(noflowR))  +      (A(noflowR) | B(noflowR)) +      (sum d : Data.(A(flow(d)) | B(flow(d))))   ).Sync;init Sync;"###, true ; "linearization_test2_various_case_29_47")]
#[test_case(r###"act a;    b,b':Nat;init a.(b(0)|b'(0))+a.(b(0)|b(0));"###, true ; "linearization_test2_various_case_30_48")]
#[test_case(r###"act a:List(List(Nat));proc X(x:List(List(Nat)))=a(x).delta;init X([[]]);"###, true ; "linearization_test2_various_case_31_49")]
#[test_case(r###"act
 a;

init
 a + tau;"###, true ; "linearization_test2_various_case_lr2plus_50")]
#[test_case(r###"act
 a;

init
 (a);"###, true ; "linearization_test2_various_case_lr2par_51")]
#[test_case(r###"sort Position = struct Full | Empty;
     Row = List(Position);

proc P(r:Row) = delta;
init P([Empty]);
"###, true ; "linearization_test2_various_case_32_52")]
#[test_case(r###"act  a:Nat; proc P(x:Nat,b:Bool,r:Real) = a(x).P(x,!b,r);      P(x:Nat) = P(x,true,1); init P(1); "###, true ; "linearization_test2_various_case_33_53")]
#[test_case(r###"act a; proc X = a.X;proc Y = sum n:Nat. X;init hide({a}, sum n:Nat. X);"###, true ; "linearization_test2_various_case_34_54")]
#[test_case(r###"act a;
init a || a;
"###, true ; "linearization_test2_various_par_55")]
#[test_case(r###"act
  c,r_dup, s_dup1, s_dup2, r_inc, s_inc, r_mul1, r_mul2, s_mul: Int;
proc
  Dup = sum x:Int. r_dup(x) | s_dup1(x) | s_dup2(x) . Dup;
  Inc = sum x:Int. r_inc(x) | s_inc(x+1) . Inc;
  Mul = sum x,y:Int. r_mul1(x) | r_mul2(y) | s_mul(x*y) . Mul;
  Dim = allow({r_dup | s_mul},
          hide({c},comm({s_dup1 | r_mul1 -> c , s_dup2 | r_inc -> c, s_inc | r_mul2 ->c},
            Dup || Inc || Mul
          ))
        );
init Dim;
"###, true ; "linearization_test2_various_gpa_10_3_56")]
#[test_case(r###"sort Bits = struct b0|b1;
     t_sys_regset_fsm_state = Bits;
     t_timer_counter_fsm_state = Bits;
map  timer_counter_fsm_state_idle: t_timer_counter_fsm_state;
act  a:Bits;
proc P(d:Bits)=a(d).delta;
glob globd:Bits;
init P(globd);
"###, true ; "linearization_test2_various_sort_aliases_57")]
#[test_case(r###"proc P=P||P; 
init P;
"###, false ; "linearization_test2_spec_58")]
#[test_case(r###"act
  c;

proc
  Q = sum b1: Bool . R;
  R = sum b1: Bool . c.delta;

init Q;
"###, true ; "linearization_test3_spec_59")]
#[test_case(r###"act a:Bool;
proc X(n:Nat)=a(n<10).Y(true);
     Y(n:Bool)=a(n).X(7);
init X(1);
"###, true ; "linearization_test4_various_case_27_60")]
#[test_case(r###"act a1,b1,c1,a2,b2,c2:Bool;
    a1,b1,c1,a2,b2,c2:Nat;

proc P1 = sum x:Bool.a1(x).b1(x).c1(x).P1+
          sum x:Nat. a1(x).b1(x).c1(x).P2;
     P2 = sum x:Bool.a2(x).b2(x).c2(x).P2+
          sum x:Nat. a2(x).b2(x).c2(x).P1;

init allow({a1,b1,c1,a2,b2,c2},P1||P2);
"###, true ; "linearization_test4_variable_clash_61")]
#[test_case(r###"act RequestReading_;
    SWFault;

map sw_fault_prob: Pos -> Real;
var x: Pos;
eqn sw_fault_prob(x) = if(x == 2, 1/20, if(x == 1, 1/10, Int2Real(0)));

proc Software(step: Pos) = (
    (dist f:Bool[if(f, sw_fault_prob(step), 1-sw_fault_prob(step))] . (f -> SWFault . delta <> (
        (step == 1) -> (
            % First step: get sensor reading
            RequestReading_ . delta
        )))));

init Software(1);
"###, true ; "linearization_test5_spec_62")]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_mcrl2_linearisation_case(text: &str, mcrl2_succeeds: bool) {
    let Ok(untyped) = UntypedProcessSpecification::parse(text) else {
        return;
    };
    let Ok(spec) = ProcessSpecification::from_untyped(untyped).and_then(|mut spec| spec.lower()) else {
        return;
    };

    match merc_linearisation::linearise(&spec, merc_linearisation::rewriter, &LinearisationOptions::default()) {
        Ok(lps) => {
            assert!(
                mcrl2_succeeds,
                "mCRL2 rejects this specification, but merc linearised it:\n{text}\n{lps}"
            );
            let mut bytes = Vec::new();
            write_lps(&lps, &mut bytes).expect("the LPS can be written");
            assert_eq!(read_lps(bytes.as_slice()).expect("the LPS can be read"), lps);
        }
        Err(LinearisationError::UnsupportedConstruct { .. }) => {}
        Err(error) => assert!(
            !mcrl2_succeeds,
            "mCRL2 linearises this specification, but merc fails:\n{text}\nerror: {error}"
        ),
    }
}
