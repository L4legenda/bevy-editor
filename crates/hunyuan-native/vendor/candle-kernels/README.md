# candle-kernels

This crate contains CUDA kernels used from candle. Some of these implementations
come from the [dfdx crate](https://github.com/coreylowman/dfdx).

This copy comes from Candle 0.9.2 at commit `e53310dd1d313157da610d9b934fc392742c52ac`.
The build keeps the unused MoE kernels in a separate Ampere library so the
tensor kernels can target Pascal. The FP16 scatter sum is compiled only for
devices with scalar FP16 atomics. Candle uses the separate `fast_sum` kernel for
tensor reductions.
