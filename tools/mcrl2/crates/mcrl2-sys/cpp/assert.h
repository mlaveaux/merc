#pragma once

#ifdef MCRL2_ENABLE_CPPTRACE
  #include <cpptrace/basic.hpp>
#endif // MCRL2_ENABLE_CPPTRACE

// Figure out the function signature macro.
#if defined(__clang__) || defined(__GNUC__)
  #define MCRL2_FUNCTION_SIGNATURE __PRETTY_FUNCTION__
#elif defined(_MSC_VER)
  #define MCRL2_FUNCTION_SIGNATURE __FUNCSIG__
#else
  #define MCRL2_FUNCTION_SIGNATURE __func__
#endif

// Special assertion macro that prints a stack trace when the assertion fails.
#if defined(MCRL2_ENABLE_CPPTRACE) && !defined(NDEBUG)

  #define MCRL2_ASSERT(x) do { \
    if (!(x)) { \
      fprintf(stderr, "%s: %s: Assertion '%s' failed\n", __FILE__, MCRL2_FUNCTION_SIGNATURE, #x); \
      if (std::getenv("RUST_BACKTRACE") != nullptr) { \
        cpptrace::generate_trace().print(); \
      } \
      std::abort(); \
    } \
  } while(0)
#else
  #define MCRL2_ASSERT(x) do { assert(x); } while(0)
#endif // MCRL2_ENABLE_CPPTRACE
