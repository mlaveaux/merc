//! Type checks every example specification from the corpus.

use std::path::Path;

use merc_syntax::UntypedProcessSpecification;
use merc_typecheck::ProcessSpecification;
use merc_utilities::check_snapshot;
use merc_utilities::test_logger;
use test_case::test_case;

/// Bump this whenever the stored snapshot format changes.
const SNAPSHOT_VERSION: u32 = 4;

#[cfg_attr(miri, ignore)]
#[test_case(include_str!("../../../examples/mCRL2/academic/abp/abp.mcrl2"), "tests/snapshot/result_abp.mcrl2" ; "abp.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/abp_bw/abp_bw.mcrl2"), "tests/snapshot/result_abp_bw.mcrl2" ; "abp_bw.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/allow/allow.mcrl2"), "tests/snapshot/result_allow.mcrl2" ; "allow.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/bakery/bakery.mcrl2"), "tests/snapshot/result_bakery.mcrl2" ; "bakery.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/bke/bke.mcrl2"), "tests/snapshot/result_bke.mcrl2" ; "bke.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/block/block.mcrl2"), "tests/snapshot/result_block.mcrl2" ; "block.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/bounded_ricart-agrawala/RA_fixed/RA_fixed_spec.mcrl2"), "tests/snapshot/result_ra_fixed_spec.mcrl2" ; "ra_fixed_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/bounded_ricart-agrawala/RA_fixed+broadcast/RA_fixed+broadcast_spec.mcrl2"), "tests/snapshot/result_ra_fixed+broadcast_spec.mcrl2" ; "ra_fixed+broadcast_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/bounded_ricart-agrawala/RA_fixed+reduced/RA_fixed+reduced_spec.mcrl2"), "tests/snapshot/result_ra_fixed+reduced_spec.mcrl2" ; "ra_fixed+reduced_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/bounded_ricart-agrawala/RA_original/RA_original_spec.mcrl2"), "tests/snapshot/result_ra_original_spec.mcrl2" ; "ra_original_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/cabp/cabp.mcrl2"), "tests/snapshot/result_cabp.mcrl2" ; "cabp.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/cellular_automata/cellular_automata.mcrl2"), "tests/snapshot/result_cellular_automata.mcrl2" ; "cellular_automata.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/commprot/commprot.mcrl2"), "tests/snapshot/result_commprot.mcrl2" ; "commprot.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/dining/dining3.mcrl2"), "tests/snapshot/result_dining3.mcrl2" ; "dining3.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/dining/dining3_cs.mcrl2"), "tests/snapshot/result_dining3_cs.mcrl2" ; "dining3_cs.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/dining/dining3_cs_seq.mcrl2"), "tests/snapshot/result_dining3_cs_seq.mcrl2" ; "dining3_cs_seq.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/dining/dining3_ns.mcrl2"), "tests/snapshot/result_dining3_ns.mcrl2" ; "dining3_ns.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/dining/dining3_ns_seq.mcrl2"), "tests/snapshot/result_dining3_ns_seq.mcrl2" ; "dining3_ns_seq.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/dining/dining3_schedule.mcrl2"), "tests/snapshot/result_dining3_schedule.mcrl2" ; "dining3_schedule.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/dining/dining3_schedule_seq.mcrl2"), "tests/snapshot/result_dining3_schedule_seq.mcrl2" ; "dining3_schedule_seq.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/dining/dining3_seq.mcrl2"), "tests/snapshot/result_dining3_seq.mcrl2" ; "dining3_seq.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/dining/dining8.mcrl2"), "tests/snapshot/result_dining8.mcrl2" ; "dining8.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/dining/dining_10.mcrl2"), "tests/snapshot/result_dining_10.mcrl2" ; "dining_10.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/food_distribution/food_package.mcrl2"), "tests/snapshot/result_food_package.mcrl2" ; "food_package.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/goback/goback.mcrl2"), "tests/snapshot/result_goback.mcrl2" ; "goback.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/hopcroft/hopcroft.mcrl2"), "tests/snapshot/result_hopcroft.mcrl2" ; "hopcroft.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/leader/dolev_klawe_rodeh.mcrl2"), "tests/snapshot/result_dolev_klawe_rodeh.mcrl2" ; "dolev_klawe_rodeh.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/leader/leader.mcrl2"), "tests/snapshot/result_leader.mcrl2" ; "leader.mcrl2")]
// #[test_case(include_str!("../../../examples/mCRL2/academic/minepump_product_line/family_based_experiments/formula1/mp_fts_prop1.mcrl2"), "tests/snapshot/result_mp_fts_prop1.mcrl2" ; "mp_fts_prop1.mcrl2")]
// #[test_case(include_str!("../../../examples/mCRL2/academic/minepump_product_line/family_based_experiments/formula10/mp_fts_prop10.mcrl2"), "tests/snapshot/result_mp_fts_prop10.mcrl2" ; "mp_fts_prop10.mcrl2")]
// #[test_case(include_str!("../../../examples/mCRL2/academic/minepump_product_line/family_based_experiments/formula11/mp_fts_prop11.mcrl2"), "tests/snapshot/result_mp_fts_prop11.mcrl2" ; "mp_fts_prop11.mcrl2")]
// #[test_case(include_str!("../../../examples/mCRL2/academic/minepump_product_line/family_based_experiments/formula12/mp_fts_prop12.mcrl2"), "tests/snapshot/result_mp_fts_prop12.mcrl2" ; "mp_fts_prop12.mcrl2")]
// #[test_case(include_str!("../../../examples/mCRL2/academic/minepump_product_line/family_based_experiments/formula2/mp_fts_prop2.mcrl2"), "tests/snapshot/result_mp_fts_prop2.mcrl2" ; "mp_fts_prop2.mcrl2")]
// #[test_case(include_str!("../../../examples/mCRL2/academic/minepump_product_line/family_based_experiments/formula3/mp_fts_prop3.mcrl2"), "tests/snapshot/result_mp_fts_prop3.mcrl2" ; "mp_fts_prop3.mcrl2")]
// #[test_case(include_str!("../../../examples/mCRL2/academic/minepump_product_line/family_based_experiments/formula4/mp_fts_prop4.mcrl2"), "tests/snapshot/result_mp_fts_prop4.mcrl2" ; "mp_fts_prop4.mcrl2")]
// #[test_case(include_str!("../../../examples/mCRL2/academic/minepump_product_line/family_based_experiments/formula5/mp_fts_prop5.mcrl2"), "tests/snapshot/result_mp_fts_prop5.mcrl2" ; "mp_fts_prop5.mcrl2")]
// #[test_case(include_str!("../../../examples/mCRL2/academic/minepump_product_line/family_based_experiments/formula6/mp_fts_prop6.mcrl2"), "tests/snapshot/result_mp_fts_prop6.mcrl2" ; "mp_fts_prop6.mcrl2")]
// #[test_case(include_str!("../../../examples/mCRL2/academic/minepump_product_line/family_based_experiments/formula7/mp_fts_prop7.mcrl2"), "tests/snapshot/result_mp_fts_prop7.mcrl2" ; "mp_fts_prop7.mcrl2")]
// #[test_case(include_str!("../../../examples/mCRL2/academic/minepump_product_line/family_based_experiments/formula8/mp_fts_prop8.mcrl2"), "tests/snapshot/result_mp_fts_prop8.mcrl2" ; "mp_fts_prop8.mcrl2")]
// #[test_case(include_str!("../../../examples/mCRL2/academic/minepump_product_line/family_based_experiments/formula9/mp_fts_prop9.mcrl2"), "tests/snapshot/result_mp_fts_prop9.mcrl2" ; "mp_fts_prop9.mcrl2")]
// #[test_case(include_str!("../../../examples/mCRL2/academic/minepump_product_line/minepump_fts.mcrl2"), "tests/snapshot/result_minepump_fts.mcrl2" ; "minepump_fts.mcrl2")]
// #[test_case(include_str!("../../../examples/mCRL2/academic/minepump_product_line/product_based_experiments/formula1/minepump.mcrl2"), "tests/snapshot/result_minepump.mcrl2" ; "minepump.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/mpsu/mpsu.mcrl2"), "tests/snapshot/result_mpsu.mcrl2" ; "mpsu.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/mutex_models/Dekker/Dekker_spec.mcrl2"), "tests/snapshot/result_dekker_spec.mcrl2" ; "dekker_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/mutex_models/Improved-mutex-naive/Improved-mutex-naive_spec.mcrl2"), "tests/snapshot/result_improved-mutex-naive_spec.mcrl2" ; "improved-mutex-naive_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/mutex_models/Mutex-naive/Mutex-naive_spec.mcrl2"), "tests/snapshot/result_mutex-naive_spec.mcrl2" ; "mutex-naive_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/mutex_models/Petersons/Petersons_spec.mcrl2"), "tests/snapshot/result_petersons_spec.mcrl2" ; "petersons_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/mutex_models/Petersons-3/Petersons-3_spec.mcrl2"), "tests/snapshot/result_petersons-3_spec.mcrl2" ; "petersons-3_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/non-atomic_registers/Aravind_BLRU/Aravind_BLRU_spec.mcrl2"), "tests/snapshot/result_aravind_blru_spec.mcrl2" ; "aravind_blru_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/non-atomic_registers/Attiya-Welch/Attiya-Welch_spec.mcrl2"), "tests/snapshot/result_attiya-welch_spec.mcrl2" ; "attiya-welch_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/non-atomic_registers/Attiya-Welch_alternate/Attiya-Welch_alternate_spec.mcrl2"), "tests/snapshot/result_attiya-welch_alternate_spec.mcrl2" ; "attiya-welch_alternate_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/non-atomic_registers/Dijkstra/Dijkstra_spec.mcrl2"), "tests/snapshot/result_dijkstra_spec.mcrl2" ; "dijkstra_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/non-atomic_registers/Knuth/Knuth_spec.mcrl2"), "tests/snapshot/result_knuth_spec.mcrl2" ; "knuth_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/non-atomic_registers/Lamport_3bit/Lamport_3bit_spec.mcrl2"), "tests/snapshot/result_lamport_3bit_spec.mcrl2" ; "lamport_3bit_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/non-atomic_registers/Lamport_3bit_incorrect_z/Lamport_3bit_incorrect_z_spec.mcrl2"), "tests/snapshot/result_lamport_3bit_incorrect_z_spec.mcrl2" ; "lamport_3bit_incorrect_z_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/non-atomic_registers/Peterson/Peterson_spec.mcrl2"), "tests/snapshot/result_peterson_spec.mcrl2" ; "peterson_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/non-atomic_registers/Register_model/Register_model_spec.mcrl2"), "tests/snapshot/result_register_model_spec.mcrl2" ; "register_model_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/non-atomic_registers/Szymanski_3bit_linear_wait/Szymanski_3bit_linear_wait_spec.mcrl2"), "tests/snapshot/result_szymanski_3bit_linear_wait_spec.mcrl2" ; "szymanski_3bit_linear_wait_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/non-atomic_registers/Szymanski_3bitlw_sem/Szymanski_3bitlw_sem_spec.mcrl2"), "tests/snapshot/result_szymanski_3bitlw_sem_spec.mcrl2" ; "szymanski_3bitlw_sem_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/non-atomic_registers/Szymanski_flag/Szymanski_flag_spec.mcrl2"), "tests/snapshot/result_szymanski_flag_spec.mcrl2" ; "szymanski_flag_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/non-atomic_registers/Szymanski_flag_with_bits/Szymanski_flag_with_bits_spec.mcrl2"), "tests/snapshot/result_szymanski_flag_with_bits_spec.mcrl2" ; "szymanski_flag_with_bits_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/non-atomic_registers/Szymanski_fwb_pe/Szymanski_fwb_pe_spec.mcrl2"), "tests/snapshot/result_szymanski_fwb_pe_spec.mcrl2" ; "szymanski_fwb_pe_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/onebit/onebit.mcrl2"), "tests/snapshot/result_onebit.mcrl2" ; "onebit.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/par/par.mcrl2"), "tests/snapshot/result_par.mcrl2" ; "par.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/parallel/parallel.mcrl2"), "tests/snapshot/result_parallel.mcrl2" ; "parallel.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/parallel_proc_with_global_var/parallel_counting.mcrl2"), "tests/snapshot/result_parallel_counting.mcrl2" ; "parallel_counting.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/peterson_justness/mutex.mcrl2"), "tests/snapshot/result_mutex.mcrl2" ; "mutex.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/producer_consumer/producer_consumer.mcrl2"), "tests/snapshot/result_producer_consumer.mcrl2" ; "producer_consumer.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/scheduler/scheduler.mcrl2"), "tests/snapshot/result_scheduler.mcrl2" ; "scheduler.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/swp/swp_fgpbp.mcrl2"), "tests/snapshot/result_swp_fgpbp.mcrl2" ; "swp_fgpbp.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/swp/swp_func.mcrl2"), "tests/snapshot/result_swp_func.mcrl2" ; "swp_func.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/swp/swp_lists.mcrl2"), "tests/snapshot/result_swp_lists.mcrl2" ; "swp_lists.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/swp/swp_with_tanenbaums_bug.mcrl2"), "tests/snapshot/result_swp_with_tanenbaums_bug.mcrl2" ; "swp_with_tanenbaums_bug.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/trains/trains.mcrl2"), "tests/snapshot/result_trains.mcrl2" ; "trains.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/academic/tree/tree.mcrl2"), "tests/snapshot/result_tree.mcrl2" ; "tree.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/beggar_my_neighbour/beggar_my_neighbour.mcrl2"), "tests/snapshot/result_beggar_my_neighbour.mcrl2" ; "beggar_my_neighbour.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/bridge_crossing/bridge_crossing.mcrl2"), "tests/snapshot/result_bridge_crossing.mcrl2" ; "bridge_crossing.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/clobber/clobber.mcrl2"), "tests/snapshot/result_clobber.mcrl2" ; "clobber.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/domineering/domineering.mcrl2"), "tests/snapshot/result_domineering.mcrl2" ; "domineering.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/four_in_a_row/four_in_a_row.mcrl2"), "tests/snapshot/result_four_in_a_row.mcrl2" ; "four_in_a_row.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/four_in_a_row_symbolic/four_in_a_row_symbolic.mcrl2"), "tests/snapshot/result_four_in_a_row_symbolic.mcrl2" ; "four_in_a_row_symbolic.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/game_of_goose/game_of_goose.mcrl2"), "tests/snapshot/result_game_of_goose.mcrl2" ; "game_of_goose.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/hex/hex.mcrl2"), "tests/snapshot/result_hex.mcrl2" ; "hex.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/knights/knights.mcrl2"), "tests/snapshot/result_knights.mcrl2" ; "knights.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/magic_square/magic_hexagon.mcrl2"), "tests/snapshot/result_magic_hexagon.mcrl2" ; "magic_hexagon.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/magic_square/magic_square.mcrl2"), "tests/snapshot/result_magic_square.mcrl2" ; "magic_square.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/open_field_tic_tac_toe/open_field_tictactoe.mcrl2"), "tests/snapshot/result_open_field_tictactoe.mcrl2" ; "open_field_tictactoe.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/othello/othello.mcrl2"), "tests/snapshot/result_othello.mcrl2" ; "othello.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/peg_solitaire/peg_solitaire.mcrl2"), "tests/snapshot/result_peg_solitaire.mcrl2" ; "peg_solitaire.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/quoridor/quoridor.mcrl2"), "tests/snapshot/result_quoridor.mcrl2" ; "quoridor.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/rubiks_cube/rubiks_cube.mcrl2"), "tests/snapshot/result_rubiks_cube.mcrl2" ; "rubiks_cube.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/rubiks_cube_small/small_cube.mcrl2"), "tests/snapshot/result_small_cube.mcrl2" ; "small_cube.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/snake/snake.mcrl2"), "tests/snapshot/result_snake.mcrl2" ; "snake.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/sokoban/sokoban.mcrl2"), "tests/snapshot/result_sokoban.mcrl2" ; "sokoban.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/sudoku/sudoku.mcrl2"), "tests/snapshot/result_sudoku.mcrl2" ; "sudoku.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/tictactoe/tictactoe.mcrl2"), "tests/snapshot/result_tictactoe.mcrl2" ; "tictactoe.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/tictactoe/tictactoe_fast.mcrl2"), "tests/snapshot/result_tictactoe_fast.mcrl2" ; "tictactoe_fast.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/games/wolf_goat_cabbage/wolf_goat_cabbage.mcrl2"), "tests/snapshot/result_wolf_goat_cabbage.mcrl2" ; "wolf_goat_cabbage.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/1394/1394-fin.mcrl2"), "tests/snapshot/result_1394-fin.mcrl2" ; "1394-fin.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/DIRAC/SMS.mcrl2"), "tests/snapshot/result_sms.mcrl2" ; "sms.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/DIRAC/WMS.mcrl2"), "tests/snapshot/result_wms.mcrl2" ; "wms.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/ERTMS/version1A/section_I/IU/ertms-hl3.mcrl2"), "tests/snapshot/result_ertms-hl3.mcrl2" ; "ertms-hl3.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/ERTMS/version1A/section_II/IU/ertms-hl3.announce.mcrl2"), "tests/snapshot/result_ertms-hl3.announce.mcrl2" ; "ertms-hl3.announce.mcrl2")]
// #[test_case(include_str!("../../../examples/mCRL2/industrial/MLV/MLV.mcrl2"), "tests/snapshot/result_mlv.mcrl2" ; "mlv.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/alma/alma.mcrl2"), "tests/snapshot/result_alma.mcrl2" ; "alma.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/brp/brp.mcrl2"), "tests/snapshot/result_brp.mcrl2" ; "brp.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/chatbox/chatbox.mcrl2"), "tests/snapshot/result_chatbox.mcrl2" ; "chatbox.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/flexray/3_Ideal_trace.expanded.mcrl2"), "tests/snapshot/result_3_ideal_trace.expanded.mcrl2" ; "3_ideal_trace.expanded.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/flexray/3_Mute_follower.expanded.mcrl2"), "tests/snapshot/result_3_mute_follower.expanded.mcrl2" ; "3_mute_follower.expanded.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/flexray/3_Mute_leader.expanded.mcrl2"), "tests/snapshot/result_3_mute_leader.expanded.mcrl2" ; "3_mute_leader.expanded.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/flexray/3_Regular.expanded.mcrl2"), "tests/snapshot/result_3_regular.expanded.mcrl2" ; "3_regular.expanded.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/flexray/Big_Deaf_follower.expanded.mcrl2"), "tests/snapshot/result_big_deaf_follower.expanded.mcrl2" ; "big_deaf_follower.expanded.mcrl2")]
// #[test_case(include_str!("../../../examples/mCRL2/industrial/garage/garage-ver.mcrl2"), "tests/snapshot/result_garage-ver.mcrl2" ; "garage-ver.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/ieee-11073/11073.mcrl2"), "tests/snapshot/result_11073.mcrl2" ; "11073.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/lift/lift3-final.mcrl2"), "tests/snapshot/result_lift3-final.mcrl2" ; "lift3-final.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/lift/lift3-init.mcrl2"), "tests/snapshot/result_lift3-init.mcrl2" ; "lift3-init.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/delta.mcrl2"), "tests/snapshot/result_delta.mcrl2" ; "delta.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/delta0.mcrl2"), "tests/snapshot/result_delta0.mcrl2" ; "delta0.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/divide2_10.mcrl2"), "tests/snapshot/result_divide2_10.mcrl2" ; "divide2_10.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/divide2_100.mcrl2"), "tests/snapshot/result_divide2_100.mcrl2" ; "divide2_100.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/divide2_500.mcrl2"), "tests/snapshot/result_divide2_500.mcrl2" ; "divide2_500.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/exists.mcrl2"), "tests/snapshot/result_exists.mcrl2" ; "exists.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/forall.mcrl2"), "tests/snapshot/result_forall.mcrl2" ; "forall.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/funccomp.mcrl2"), "tests/snapshot/result_funccomp.mcrl2" ; "funccomp.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/gpa_10_1.mcrl2"), "tests/snapshot/result_gpa_10_1.mcrl2" ; "gpa_10_1.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/gpa_10_2.mcrl2"), "tests/snapshot/result_gpa_10_2.mcrl2" ; "gpa_10_2.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/gpa_10_3.mcrl2"), "tests/snapshot/result_gpa_10_3.mcrl2" ; "gpa_10_3.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/lambda.mcrl2"), "tests/snapshot/result_lambda.mcrl2" ; "lambda.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/list.mcrl2"), "tests/snapshot/result_list.mcrl2" ; "list.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/numbers.mcrl2"), "tests/snapshot/result_numbers.mcrl2" ; "numbers.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/rational.mcrl2"), "tests/snapshot/result_rational.mcrl2" ; "rational.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/sets_bags.mcrl2"), "tests/snapshot/result_sets_bags.mcrl2" ; "sets_bags.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/small1.mcrl2"), "tests/snapshot/result_small1.mcrl2" ; "small1.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/small2.mcrl2"), "tests/snapshot/result_small2.mcrl2" ; "small2.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/small3.mcrl2"), "tests/snapshot/result_small3.mcrl2" ; "small3.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/tau.mcrl2"), "tests/snapshot/result_tau.mcrl2" ; "tau.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/time.mcrl2"), "tests/snapshot/result_time.mcrl2" ; "time.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/upcast.mcrl2"), "tests/snapshot/result_upcast.mcrl2" ; "upcast.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/probabilistic/airplane_ticket/airplane_ticket.mcrl2"), "tests/snapshot/result_airplane_ticket.mcrl2" ; "airplane_ticket.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/probabilistic/ant_on_grid/ant_on_grid.mcrl2"), "tests/snapshot/result_ant_on_grid.mcrl2" ; "ant_on_grid.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/probabilistic/coin_tossing/coins.mcrl2"), "tests/snapshot/result_coins.mcrl2" ; "coins.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/probabilistic/coins_simulate_dice/dice.mcrl2"), "tests/snapshot/result_dice.mcrl2" ; "dice.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/probabilistic/game_of_goose/game_of_goose_stochastic.mcrl2"), "tests/snapshot/result_game_of_goose_stochastic.mcrl2" ; "game_of_goose_stochastic.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/probabilistic/monty_hall_tv_show/monty_hall.mcrl2"), "tests/snapshot/result_monty_hall.mcrl2" ; "monty_hall.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/probabilistic/self_stabilisation/self_stabilisation.mcrl2"), "tests/snapshot/result_self_stabilisation.mcrl2" ; "self_stabilisation.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/probabilistic/shared_coin_protocol/shared_coin_protocol.mcrl2"), "tests/snapshot/result_shared_coin_protocol.mcrl2" ; "shared_coin_protocol.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/probabilistic/slot_machines/1slot/1slot_spec.mcrl2"), "tests/snapshot/result_1slot_spec.mcrl2" ; "1slot_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/probabilistic/slot_machines/3slot/3slot_spec.mcrl2"), "tests/snapshot/result_3slot_spec.mcrl2" ; "3slot_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/probabilistic/slot_machines/3slot_hold/3slot_hold_spec.mcrl2"), "tests/snapshot/result_3slot_hold_spec.mcrl2" ; "3slot_hold_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/probabilistic/slot_machines/3slot_hold/3slot_hold_spec_average.mcrl2"), "tests/snapshot/result_3slot_hold_spec_average.mcrl2" ; "3slot_hold_spec_average.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/probabilistic/slot_machines/paylines/10_paylines_game_spec.mcrl2"), "tests/snapshot/result_10_paylines_game_spec.mcrl2" ; "10_paylines_game_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/probabilistic/slot_machines/paylines/5_paylines_game_spec.mcrl2"), "tests/snapshot/result_5_paylines_game_spec.mcrl2" ; "5_paylines_game_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/probabilistic/slot_machines/reels_game/reels_game_spec.mcrl2"), "tests/snapshot/result_reels_game_spec.mcrl2" ; "reels_game_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/probabilistic/spinning_mule_woolhouse/spinning_mule.mcrl2"), "tests/snapshot/result_spinning_mule.mcrl2" ; "spinning_mule.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/probabilistic/spinning_mule_woolhouse/spinning_mule_optimized.mcrl2"), "tests/snapshot/result_spinning_mule_optimized.mcrl2" ; "spinning_mule_optimized.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/probabilistic/spinning_mule_woolhouse/spinning_mule_woolhouse.mcrl2"), "tests/snapshot/result_spinning_mule_woolhouse.mcrl2" ; "spinning_mule_woolhouse.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/probabilistic/sultan_of_persia/sultan_of_persia.mcrl2"), "tests/snapshot/result_sultan_of_persia.mcrl2" ; "sultan_of_persia.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/project/wafer_stepper/wafer_stepper.mcrl2"), "tests/snapshot/result_wafer_stepper.mcrl2" ; "wafer_stepper.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/software_models/Knuths_dancing_links/Dancing_links/Dancing_links_spec.mcrl2"), "tests/snapshot/result_dancing_links_spec.mcrl2" ; "dancing_links_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/software_models/Knuths_dancing_links/Dancing_links_no_stack/Dancing_links_no_stack_spec.mcrl2"), "tests/snapshot/result_dancing_links_no_stack_spec.mcrl2" ; "dancing_links_no_stack_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/software_models/Knuths_dancing_links/Dancing_links_remove_0/Dancing_links_remove_0_spec.mcrl2"), "tests/snapshot/result_dancing_links_remove_0_spec.mcrl2" ; "dancing_links_remove_0_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/software_models/Lamport_queue/Lamport_queue_spec.mcrl2"), "tests/snapshot/result_lamport_queue_spec.mcrl2" ; "lamport_queue_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/software_models/Petersons_mutex/Petersons_F_F/Petersons_F_F_spec.mcrl2"), "tests/snapshot/result_petersons_f_f_spec.mcrl2" ; "petersons_f_f_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/software_models/Petersons_mutex/Petersons_F_T/Petersons_F_T_spec.mcrl2"), "tests/snapshot/result_petersons_f_t_spec.mcrl2" ; "petersons_f_t_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/software_models/Petersons_mutex/Petersons_T_T/Petersons_T_T_spec.mcrl2"), "tests/snapshot/result_petersons_t_t_spec.mcrl2" ; "petersons_t_t_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/software_models/Treiber_stack/Treiber_CAS/Treiber_CAS_spec.mcrl2"), "tests/snapshot/result_treiber_cas_spec.mcrl2" ; "treiber_cas_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/software_models/Treiber_stack/Treiber_DCAS/Treiber_DCAS_spec.mcrl2"), "tests/snapshot/result_treiber_dcas_spec.mcrl2" ; "treiber_dcas_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/software_models/Treiber_stack/Treiber_no_CAS/Treiber_no_CAS_spec.mcrl2"), "tests/snapshot/result_treiber_no_cas_spec.mcrl2" ; "treiber_no_cas_spec.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/timed/ball_game/ball_game.mcrl2"), "tests/snapshot/result_ball_game.mcrl2" ; "ball_game.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/timed/clock/clock_drift.mcrl2"), "tests/snapshot/result_clock_drift.mcrl2" ; "clock_drift.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/timed/clock/clock_exact.mcrl2"), "tests/snapshot/result_clock_exact.mcrl2" ; "clock_exact.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/timed/clock/clock_hasty.mcrl2"), "tests/snapshot/result_clock_hasty.mcrl2" ; "clock_hasty.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/timed/fischer/fischer.mcrl2"), "tests/snapshot/result_fischer.mcrl2" ; "fischer.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/timed/light/light.mcrl2"), "tests/snapshot/result_light.mcrl2" ; "light.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/timed/simple/simple.mcrl2"), "tests/snapshot/result_simple.mcrl2" ; "simple.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/visualisation/carpet/carpet.mcrl2"), "tests/snapshot/result_carpet.mcrl2" ; "carpet.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/visualisation/cube/cube.mcrl2"), "tests/snapshot/result_cube.mcrl2" ; "cube.mcrl2")]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_typecheck_mcrl2_spec(input: &str, snapshot_file: &str) {
    test_logger();

    let spec = UntypedProcessSpecification::parse(input).expect("the example corpus parses in merc_syntax");
    match ProcessSpecification::from_untyped(spec) {
        Ok(typed) => {
            check_snapshot(
                &typed.data_specification().to_typed_string(),
                Path::new(snapshot_file),
                SNAPSHOT_VERSION,
            )
            .expect("Could not read or write the tests/snapshot file");
        }
        Err(err) => panic!("{err}"),
    }
}

/// Corpus files whose structs happen to declare unrelated, same-named constructors/projections.
#[test_case(include_str!("../../../examples/mCRL2/industrial/garage/garage.mcrl2") ; "garage.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/garage/garage-r1.mcrl2") ; "garage-r1.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/garage/garage-r2.mcrl2") ; "garage-r2.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/garage/garage-r2-error.mcrl2") ; "garage-r2-error.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/industrial/garage/garage-r3.mcrl2") ; "garage-r3.mcrl2")]
#[test_case(include_str!("../../../examples/mCRL2/language/struct.mcrl2") ; "struct.mcrl2")]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_typecheck_mcrl2_spec_rejected_by_pooled_struct_signature(input: &str) {
    test_logger();

    let spec = UntypedProcessSpecification::parse(input).expect("the example corpus parses in merc_syntax");
    match ProcessSpecification::from_untyped(spec) {
        Ok(_) => panic!("expected a same-named unrelated struct declaration to make this ambiguous"),
        Err(err) => assert!(
            err.to_string().contains("ambiguous"),
            "expected an ambiguity error, got: {err}"
        ),
    }
}
