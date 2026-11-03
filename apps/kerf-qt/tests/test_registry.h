#pragma once

// How a test class joins kerf-qt-tests: each tests/*_test.cpp ends with
// KERF_TEST_CLASS(kerf::SomeTest), and tests/test_main.cpp runs every class
// registered so, in name order, each with QTest::qExec.

#include <QMetaObject>
#include <QObject>

#include <memory>

namespace kerf::test {

using TestFactory = std::unique_ptr<QObject> (*)();

// Adds a test class to the run. The class is built only when it runs, after
// the application and the test settings exist.
bool registerTestClass(const QMetaObject *meta, TestFactory make);

} // namespace kerf::test

#define KERF_TEST_CLASS(Class)                                                                                         \
    namespace {                                                                                                        \
    [[maybe_unused]] const bool kerfTestClassRegistered = ::kerf::test::registerTestClass(                             \
        &Class::staticMetaObject, []() -> std::unique_ptr<QObject> { return std::make_unique<Class>(); });             \
    }
