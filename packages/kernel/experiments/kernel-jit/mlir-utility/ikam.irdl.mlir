// E342: a small dialect of ikam's own, declared at run time with IRDL (no
// C++ and no rebuild of MLIR): `ikam.dot` takes two values of one type and
// returns an f32 score.
irdl.dialect @ikam {
  irdl.operation @dot {
    %v = irdl.any
    %f = irdl.is f32
    irdl.operands(lhs: %v, rhs: %v)
    irdl.results(score: %f)
  }
}
