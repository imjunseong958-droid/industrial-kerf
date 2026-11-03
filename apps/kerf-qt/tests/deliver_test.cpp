// Tests for the Deliver tray. Its package fills this file in; until then one
// slot keeps the class runnable.

#include "test_registry.h"

#include <QObject>
#include <QSettings>
#include <QTest>

namespace kerf {

class DeliverTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    void deliverTrayIsNotBuiltYet() { QVERIFY(true); }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::DeliverTest)

#include "deliver_test.moc"
