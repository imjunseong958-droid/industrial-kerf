// Tests for ui/app_icon.h: the editor carries Kerf's icon at each size it
// names, and each is the picture tools/kerf-icon rendered for that size.

#include "app_icon.h"
#include "test_registry.h"

#include <QColor>
#include <QFile>
#include <QIcon>
#include <QImage>
#include <QObject>
#include <QSize>
#include <QString>
#include <QTest>

namespace kerf {

class AppIconTest final : public QObject {
    Q_OBJECT

private slots:
    // Every size is compiled in, at its own size, and is the file in
    // resources/icon, so what a window shows is what the packages install.
    void theIconIsCompiledInAtEverySize() {
        const QIcon icon = appIcon();
        QVERIFY(!icon.isNull());
        const QList<QSize> sizes = icon.availableSizes();
        for (const int side : appIconSides()) {
            QVERIFY2(sizes.contains(QSize(side, side)), qPrintable(QStringLiteral("no %1 px icon").arg(side)));
            const QString name = QStringLiteral("industrial-kerf-%1.png").arg(side);
            QFile compiled(QStringLiteral(":/icon/") + name);
            QFile source(QStringLiteral(KERF_QT_SOURCE_DIR "/resources/icon/") + name);
            QVERIFY2(compiled.open(QIODevice::ReadOnly), qPrintable(name));
            QVERIFY2(source.open(QIODevice::ReadOnly), qPrintable(name));
            QCOMPARE(compiled.readAll(), source.readAll());
        }
    }

    // At 16 px, the size of a window's title bar, the icon is still Kerf's: a
    // solid tile with clear corners, the crane's red crown and the violet clip.
    void theSmallestIconIsStillKerfs() {
        const QImage image = appIcon().pixmap(QSize(16, 16)).toImage().convertToFormat(QImage::Format_ARGB32);
        QCOMPARE(image.size(), QSize(16, 16));
        QCOMPARE(image.pixelColor(0, 0).alpha(), 0);
        QCOMPARE(image.pixelColor(8, 14).alpha(), 255);
        const QColor crown = image.pixelColor(10, 3);
        QVERIFY2(crown.red() > 180 && crown.green() < 130 && crown.blue() < 130, qPrintable(crown.name()));
        const QColor clip = image.pixelColor(13, 11);
        QVERIFY2(clip.blue() > clip.green() && clip.blue() > clip.red(), qPrintable(clip.name()));
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::AppIconTest)

#include "app_icon_test.moc"
