cmake_minimum_required(VERSION 3.21)
project(cut_qt VERSION 0.1 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)
set(CMAKE_AUTOUIC ON)

find_package(Qt6 REQUIRED COMPONENTS Widgets)

set(CUT_BRIDGE_LIBRARY "${CMAKE_CURRENT_LIST_DIR}/../../target/debug/libcut_qt_bridge${CMAKE_SHARED_LIBRARY_SUFFIX}"
    CACHE FILEPATH "Path to the Rust cut_qt_bridge cdylib")
if(NOT EXISTS "${CUT_BRIDGE_LIBRARY}")
    message(WARNING "Rust bridge was not found at ${CUT_BRIDGE_LIBRARY}; build cut_qt_bridge before linking.")
endif()

add_executable(cut-qt main.cpp)
target_include_directories(cut-qt PRIVATE ${CMAKE_CURRENT_LIST_DIR})
target_link_libraries(cut-qt PRIVATE Qt6::Widgets "${CUT_BRIDGE_LIBRARY}")
