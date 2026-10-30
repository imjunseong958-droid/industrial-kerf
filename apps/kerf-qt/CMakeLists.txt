cmake_minimum_required(VERSION 3.21)
project(kerf_qt VERSION 0.1 LANGUAGES CXX)

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
if(APPLE)
    # Qt 6.11's moc follows framework headers to Q_OS_UNIX but not to
    # Q_OS_MACOS, so it reads qtads' floating container as the Linux
    # QDockWidget while the compiler builds the macOS QWidget.
    set_property(TARGET qtadvanceddocking-qt6 APPEND PROPERTY AUTOMOC_MOC_OPTIONS -DQ_OS_MACOS)
endif()

set(KERF_BRIDGE_LIBRARY "${CMAKE_CURRENT_LIST_DIR}/../../target/debug/libkerf_qt_bridge${CMAKE_SHARED_LIBRARY_SUFFIX}"
    CACHE FILEPATH "Path to the Rust kerf_qt_bridge cdylib")
if(NOT EXISTS "${KERF_BRIDGE_LIBRARY}")
    message(WARNING "Rust bridge was not found at ${KERF_BRIDGE_LIBRARY}; build kerf_qt_bridge before linking.")
endif()

set(KERF_QT_SOURCES
    ask_prompt.cpp
    ask_prompt.h
    bridge.h
    command_registry.cpp
    command_registry.h
    editor_common.cpp
    editor_common.h
    editor_style.h
    effect_controls_panel.cpp
    effect_controls_panel.h
    effects_panel.cpp
    effects_panel.h
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

add_executable(kerf-qt main.cpp ${KERF_QT_SOURCES})
target_include_directories(kerf-qt PRIVATE ${CMAKE_CURRENT_LIST_DIR})
target_link_libraries(kerf-qt PRIVATE Qt6::Widgets ads::qtadvanceddocking-qt6 "${KERF_BRIDGE_LIBRARY}")

option(KERF_QT_BUILD_TESTS "Build Qt behavior tests" OFF)
if(KERF_QT_BUILD_TESTS)
    include(CTest)
    find_package(Qt6 REQUIRED COMPONENTS Test)
    add_executable(kerf-qt-tests editor_test.cpp ${KERF_QT_SOURCES})
    target_compile_definitions(kerf-qt-tests PRIVATE KERF_QT_TESTING)
    target_include_directories(kerf-qt-tests PRIVATE ${CMAKE_CURRENT_LIST_DIR})
    target_link_libraries(kerf-qt-tests PRIVATE Qt6::Widgets Qt6::Test ads::qtadvanceddocking-qt6 "${KERF_BRIDGE_LIBRARY}")
    add_test(NAME kerf-qt-ui COMMAND kerf-qt-tests -platform offscreen)
endif()
