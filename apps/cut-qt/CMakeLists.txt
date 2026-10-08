cmake_minimum_required(VERSION 3.21)
project(cut_qt VERSION 0.1 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)
set(CMAKE_AUTOUIC ON)

find_package(Qt6 REQUIRED COMPONENTS Widgets)

# Docking: Qt Advanced Docking System (LGPL-2.1), built from a pinned
# release and linked as a shared library. Set
# FETCHCONTENT_SOURCE_DIR_QTADS to build from a local checkout instead.
include(FetchContent)
set(QT_VERSION_MAJOR 6)
set(BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
set(ADS_VERSION 5.1.1)
FetchContent_Declare(qtads
    GIT_REPOSITORY https://github.com/githubuser0xFFFF/Qt-Advanced-Docking-System.git
    GIT_TAG v5.1.1
    GIT_SHALLOW TRUE)
FetchContent_MakeAvailable(qtads)

set(CUT_BRIDGE_LIBRARY "${CMAKE_CURRENT_LIST_DIR}/../../target/debug/libcut_qt_bridge${CMAKE_SHARED_LIBRARY_SUFFIX}"
    CACHE FILEPATH "Path to the Rust cut_qt_bridge cdylib")
if(NOT EXISTS "${CUT_BRIDGE_LIBRARY}")
    message(WARNING "Rust bridge was not found at ${CUT_BRIDGE_LIBRARY}; build cut_qt_bridge before linking.")
endif()

set(CUT_QT_SOURCES
    bridge.h
    command_registry.cpp
    command_registry.h
    editor_common.cpp
    editor_common.h
    editor_style.h
    effect_controls_panel.cpp
    effect_controls_panel.h
    main_window.cpp
    main_window.h
    media_artwork.cpp
    media_artwork.h
    project_panel.cpp
    project_panel.h
    source_monitor_panel.cpp
    source_monitor_panel.h
    timeline_widget.cpp
    timeline_widget.h
    viewer_widget.cpp
    viewer_widget.h
    workspace.cpp
    workspace.h)

add_executable(cut-qt main.cpp ${CUT_QT_SOURCES})
target_include_directories(cut-qt PRIVATE ${CMAKE_CURRENT_LIST_DIR})
target_link_libraries(cut-qt PRIVATE Qt6::Widgets ads::qtadvanceddocking-qt6 "${CUT_BRIDGE_LIBRARY}")

option(CUT_QT_BUILD_TESTS "Build Qt behavior tests" OFF)
if(CUT_QT_BUILD_TESTS)
    include(CTest)
    find_package(Qt6 REQUIRED COMPONENTS Test)
    add_executable(cut-qt-tests editor_test.cpp ${CUT_QT_SOURCES})
    target_compile_definitions(cut-qt-tests PRIVATE CUT_QT_TESTING)
    target_include_directories(cut-qt-tests PRIVATE ${CMAKE_CURRENT_LIST_DIR})
    target_link_libraries(cut-qt-tests PRIVATE Qt6::Widgets Qt6::Test ads::qtadvanceddocking-qt6 "${CUT_BRIDGE_LIBRARY}")
    add_test(NAME cut-qt-ui COMMAND cut-qt-tests -platform offscreen)
endif()
