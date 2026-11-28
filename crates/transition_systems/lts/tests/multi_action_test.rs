use merc_collections::VecBag;
use merc_lts::LtsAction;
use merc_lts::LtsMultiAction;
use merc_lts::TransitionLabel;

#[test]
fn test_multi_action_aterm_round_trip() {
    let actions = [
        LtsMultiAction::new(VecBag::singleton(LtsAction::new("a".to_string(), vec![]))),
        LtsMultiAction::<LtsAction>::tau_label(),
    ];

    for multi_action in actions {
        let term = multi_action.to_mcrl2_aterm().unwrap();
        let back = LtsMultiAction::from_mcrl2_aterm(term).unwrap();
        assert_eq!(back, multi_action);
    }
}
