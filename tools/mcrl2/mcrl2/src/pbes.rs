use std::marker::PhantomData;

use mcrl2_sys::cxx::UniquePtr;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_stategraph_local_algorithm_run;
use mcrl2_sys::pbes::ffi::mcrl2_unify_parameters;
use mcrl2_sys::pbes::ffi::pbes;
use mcrl2_sys::pbes::ffi::srf_pbes;
use mcrl2_sys::pbes::ffi::stategraph_algorithm;
use merc_utilities::MercError;

pub struct Pbes {
    pbes: UniquePtr<pbes>,
}

impl Pbes {
    /// Load a PBES from a file.
    pub fn from_file(filename: &str) -> Result<Self, MercError> {
        Ok(Pbes {
            pbes: mcrl2_sys::pbes::ffi::mcrl2_load_pbes_from_file(filename)?,
        })
    }
}

impl From<Pbes> for PbesSrf {
    fn from(pbes: Pbes) -> Self {
        PbesSrf::from(&pbes).unwrap()
    }
}

impl fmt::Display for Pbes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.pbes.as_ref().unwrap())
    }
}

pub struct PbesStategraph {
    algorithm: UniquePtr<stategraph_algorithm>,
    control_flow_graphs: Vec<ControlFlowGraph>,
}

impl PbesStategraph {
    /// Run the state graph algorithm on the given PBES.
    pub fn run(pbes: &Pbes) -> Result<Self, MercError> {
        let algorithm = mcrl2_pbes_stategraph_local_algorithm_run(&pbes.pbes)?;

        // let control_flow_graphs: (0..mcrl2_pbes_stategraph_local_algorithm_num_of_cfgs(&algorithm)?)
        //     .map(|index| ControlFlowGraph { index })
        //     .collect();

        Ok(PbesStategraph {
            algorithm,
            control_flow_graphs: Vec::new(),
        })
    }

    /// Returns the control flow graphs identified by the algorithm.
    pub fn control_flow_graphs(&self) -> Vec<PbesStategraphControlFlowGraph> {
        unimplemented!()
    }
}

/// Represents a local control flow graph identified by the PBES state graph algorithm.
pub struct PbesStategraphControlFlowGraph {
    index: usize,
}

/// Represents a PBES in SRF form.
pub struct PbesSrf {
    srf_pbes: UniquePtr<srf_pbes>,
}

impl PbesSrf {
    /// Convert a PBES to an SRF PBES.
    pub fn from(pbes: &Pbes) -> Result<Self, MercError> {
        Ok(PbesSrf {
            srf_pbes: mcrl2_sys::pbes::ffi::mcrl2_pbes_to_srf_pbes(&pbes.pbes)?,
        })
    }

    /// Unify all parameters of the equations.
    pub fn unify_parameters(&mut self, ignore_ce_equations: bool, reset: bool) -> Result<(), MercError> {
        mcrl2_unify_parameters(self.srf_pbes.pin_mut(), ignore_ce_equations, reset)?;
        Ok(())
    }

    /// Returns the srf equations of the SRF pbes.
    pub fn equations(&self) -> Vec<PbesSrfEquation> {
        unimplemented!()
    }
}

pub struct PbesSrfEquation {}

impl PbesSrfEquation {
    /// Returns the parameters of the equation.
    pub fn parameters(&self) -> Vec<Mcrl2AtermList<Mcrl2ATerm>> {
        unimplemented!()
    }
}
