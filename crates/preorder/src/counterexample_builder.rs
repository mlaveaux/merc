

/// 
/// This struct can be used to generate trace counterer examples, e.g, the ones
/// produced for failures refinement.
/// 
/// # Details
/// 
/// A counterexample is a trace from the root. The data structure to construct
/// such a trace is a tree pointing to the root where each branch is labelled
/// with an action. By walking from a leaf to the root the desired
/// counterexample can be constructed.
struct CounterExampleBuilder {

}