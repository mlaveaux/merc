add_executable(merc_example
  main.cpp
)

target_link_libraries(merc_example
  PRIVATE
    merc_ffi
)