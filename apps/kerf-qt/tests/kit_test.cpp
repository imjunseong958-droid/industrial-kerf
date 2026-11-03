// Tests for the shared UI kit. Its package fills this file in; until then one
// slot keeps the class runnable.

#include "test_registry.h"

#include <QObject>
#include <QSettings>
#include <QTest>

namespace kerf {

class KitTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    void kitIsNotBuiltYet() { QVERIFY(true); }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::KitTest)

#include "kit_test.moc"
