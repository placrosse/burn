use cubecl::prelude::*;

use crate::kernel::reduce::Argmax;

use super::base::{ReduceDimNaive, ReduceDimNaiveFamily};

impl ReduceDimNaiveFamily for Argmax {
    type Reduce<E: Numeric> = Self;
}

#[cube]
impl<EI: Numeric> ReduceDimNaive<EI> for Argmax {
    type Accumulator = (EI, u32);

    fn initialize_naive() -> Self::Accumulator {
        // TODO: switch to using f32::NEG_INFINITY when it's supported: https://github.com/tracel-ai/cubecl/issues/68
        (EI::min_value(), 0u32)
    }

    fn inner_loop_naive(accumulator: &mut Self::Accumulator, current_value: EI, i: u32) {
        let (max, index) = accumulator;
        if current_value > *max {
            *max = current_value;
            *index = i;
        }
    }

    fn assign_naive<EO: Numeric>(
        output: &mut Tensor<EO>,
        accumulator: Self::Accumulator,
        _shape_reduce_dim: u32,
    ) {
        let (_, index) = accumulator;
        output[ABSOLUTE_POS] = EO::cast_from(index);
    }
}
