cmake_minimum_required(VERSION 3.21)
project(kerf_qt VERSION 0.1 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)
set(CMAKE_AUTOUIC ON)
if(MSVC)
    # The sources are UTF-8; MSVC otherwise reads them in the system code page.
    add_compile_options(/utf-8)
endif()

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

if(WIN32)
    # MSVC links a DLL through its import library, which Cargo writes beside
    # it as kerf_qt_bridge.dll.lib.
    set(KERF_BRIDGE_DEFAULT "${CMAKE_CURRENT_LIST_DIR}/../../target/debug/kerf_qt_bridge.dll.lib")
else()
    set(KERF_BRIDGE_DEFAULT "${CMAKE_CURRENT_LIST_DIR}/../../target/debug/libkerf_qt_bridge${CMAKE_SHARED_LIBRARY_SUFFIX}")
endif()
set(KERF_BRIDGE_LIBRARY "${KERF_BRIDGE_DEFAULT}"
    CACHE FILEPATH "Path to the Rust kerf_qt_bridge cdylib (on Windows, its .dll.lib import library)")
if(NOT EXISTS "${KERF_BRIDGE_LIBRARY}")
    message(WARNING "Rust bridge was not found at ${KERF_BRIDGE_LIBRARY}; build kerf_qt_bridge before linking.")
endif()

# Windows looks for a program's DLLs beside it: copy there the bridge and
# the docking library built here. Qt's own come from its bin folder on PATH.
function(kerf_copy_runtime_dlls target)
    if(WIN32)
        string(REGEX REPLACE "\\.lib$" "" bridge_dll "${KERF_BRIDGE_LIBRARY}")
        add_custom_command(TARGET ${target} POST_BUILD
            COMMAND ${CMAKE_COMMAND} -E copy_if_different
                "${bridge_dll}" "$<TARGET_FILE:qtadvanceddocking-qt6>" "$<TARGET_FILE_DIR:${target}>"
            VERBATIM)
    endif()
endfunction()

set(KERF_QT_SOURCES
    ask_prompt.cpp
    ask_prompt.h
    bridge.h
    chat_panel.cpp
    chat_panel.h
    command_registry.cpp
    command_registry.h
    device_frame.cpp
    device_frame.h
    editor_common.cpp
    editor_common.h
    editor_style.h
    editor_theme.cpp
    editor_theme.h
    effects_panel.cpp
    effects_panel.h
    inspector_panel.cpp
    inspector_panel.h
    inspector_presentation.cpp
    inspector_presentation.h
    inspector_widgets.cpp
    inspector_widgets.h
    launcher_style.h
    main_window.cpp
    main_window.h
    media_artwork.cpp
    media_artwork.h
    player_overlay.cpp
    player_overlay.h
    presets_panel.cpp
    presets_panel.h
    project_launcher.cpp
    project_launcher.h
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
if(APPLE)
    # The launcher and the editor draw their own titles under AppKit's
    # transparent title bar, beside its window buttons.
    enable_language(OBJCXX)
    list(APPEND KERF_QT_SOURCES macos_window.h macos_window.mm)
    set(KERF_QT_PLATFORM_LIBRARIES "-framework AppKit")
endif()

# The sources the editor and its tests share, compiled once for both. The
# window and the launcher behave differently under KERF_QT_TESTING, so each
# executable compiles its own; their headers go with them, since AUTOMOC
# runs moc on a source's header in whichever target compiles the source.
set(KERF_QT_VARIANT_SOURCES main_window.cpp main_window.h project_launcher.cpp project_launcher.h)
set(KERF_QT_SHARED_SOURCES ${KERF_QT_SOURCES})
list(REMOVE_ITEM KERF_QT_SHARED_SOURCES ${KERF_QT_VARIANT_SOURCES})
add_library(kerf-qt-shared OBJECT ${KERF_QT_SHARED_SOURCES})
target_include_directories(kerf-qt-shared PUBLIC ${CMAKE_CURRENT_LIST_DIR})
target_link_libraries(kerf-qt-shared PUBLIC Qt6::Widgets ads::qtadvanceddocking-qt6 "${KERF_BRIDGE_LIBRARY}"
    ${KERF_QT_PLATFORM_LIBRARIES})

add_executable(kerf-qt main.cpp ${KERF_QT_VARIANT_SOURCES})
if(APPLE)
    # A macOS application bundle (target/qt/kerf-qt.app), so the system
    # treats the editor as an app of its own: its name in the menu bar and
    # the Dock, and an identifier other apps and tools can address it by.
    set_target_properties(kerf-qt PROPERTIES
        MACOSX_BUNDLE TRUE
        MACOSX_BUNDLE_INFO_PLIST "${CMAKE_CURRENT_LIST_DIR}/Info.plist.in"
        MACOSX_BUNDLE_GUI_IDENTIFIER "com.industrial-kerf.editor"
        MACOSX_BUNDLE_BUNDLE_NAME "Industrial Kerf"
        MACOSX_BUNDLE_SHORT_VERSION_STRING "${PROJECT_VERSION}"
        MACOSX_BUNDLE_BUNDLE_VERSION "${PROJECT_VERSION}")
endif()
target_link_libraries(kerf-qt PRIVATE kerf-qt-shared)
kerf_copy_runtime_dlls(kerf-qt)

option(KERF_QT_BUILD_TESTS "Build Qt behavior tests" OFF)
if(KERF_QT_BUILD_TESTS)
    include(CTest)
    find_package(Qt6 REQUIRED COMPONENTS Test)
    add_executable(kerf-qt-tests editor_test.cpp test_footage.cpp ${KERF_QT_VARIANT_SOURCES})
    target_compile_definitions(kerf-qt-tests PRIVATE KERF_QT_TESTING)
    target_link_libraries(kerf-qt-tests PRIVATE kerf-qt-shared Qt6::Test)
    kerf_copy_runtime_dlls(kerf-qt-tests)
    add_test(NAME kerf-qt-ui COMMAND kerf-qt-tests -platform offscreen)
endif()
