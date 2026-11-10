#pragma once

// The Qt test classes (apps/kerf-qt/tests) that may reach into a widget's
// private members. A widget names them all with KERF_TEST_FRIENDS where it
// would name one test class as a friend. A new test class that reaches into
// private members is added here as well as in its own tests/<name>_test.cpp;
// one that needs no private access (TokensTest) is not.

namespace kerf {

class BinBrowserTest;
class BusTest;
class CommentsTest;
class MotionPathTest;
class KitTest;
class LauncherTest;
class LinkTest;
class MockupParityTest;
class MotionTest;
class ReviewModelTest;
class StageShellTest;
class StageTest;
class StatusTest;
class ThemeTest;
class TimelineTest;
class ToolbarTest;
class WiringTest;

namespace test {
// The shared fixtures' window into private members (tests/fixtures.cpp).
struct Access;
} // namespace test

} // namespace kerf

#define KERF_TEST_FRIENDS                                                                                              \
    friend class ::kerf::BinBrowserTest;                                                                               \
    friend class ::kerf::BusTest;                                                                                      \
    friend class ::kerf::CommentsTest;                                                                                 \
    friend class ::kerf::MotionPathTest;                                                                               \
    friend class ::kerf::KitTest;                                                                                      \
    friend class ::kerf::LauncherTest;                                                                                 \
    friend class ::kerf::LinkTest;                                                                                     \
    friend class ::kerf::MockupParityTest;                                                                             \
    friend class ::kerf::MotionTest;                                                                                    \
    friend class ::kerf::ReviewModelTest;                                                                              \
    friend class ::kerf::StageShellTest;                                                                                \
    friend class ::kerf::StageTest;                                                                                    \
    friend class ::kerf::StatusTest;                                                                                   \
    friend class ::kerf::ThemeTest;                                                                                    \
    friend class ::kerf::TimelineTest;                                                                                 \
    friend class ::kerf::ToolbarTest;                                                                                  \
    friend class ::kerf::WiringTest;                                                                                   \
                                                                                    \
    friend struct ::kerf::test::Access;
