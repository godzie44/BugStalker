mod common;

mod breakpoints;
mod io;
mod multithreaded;
mod steps;
mod symbol;
mod variables;

const HW_APP: &str = "./tests/hello_world";
const CALC_APP: &str = "./tests/calc";
const MT_APP: &str = "./target/debug/mt";
const VARS_APP: &str = "./target/debug/vars";
