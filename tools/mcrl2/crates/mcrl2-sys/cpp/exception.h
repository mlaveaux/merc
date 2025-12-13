#pragma once

#include <cstdlib>

#ifdef MCRL2_ENABLE_CPPTRACE
  #include <cpptrace/from_current.hpp>
#endif // MCRL2_ENABLE_CPPTRACE

namespace rust::behavior {

// Define a try-catch block that catches C++ exceptions with proper stack traces. Otherwise, we simply
// let exceptions propagate normally. Meaning they will be converted to Rust `Result` without stack traces.
#ifdef MCRL2_ENABLE_CPPTRACE
  template <typename Try, typename Fail>
  static void trycatch(Try &&func, Fail &&fail) noexcept 
  { 
    CPPTRACE_TRY {
      func();
    } CPPTRACE_CATCH(const std::exception &e) {
      if (std::getenv("RUST_BACKTRACE") != nullptr) {
        cpptrace::from_current_exception().print();
      }

      fail(e.what());
    }
  }
#endif // MCRL2_ENABLE_CPPTRACE

} // namespace rust::behaviour