// Tests for the toolbar: format bank and Export.

#include "fixtures.h"
#include "test_registry.h"

#include "ask_prompt.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "device_frame.h"
#include "editor_common.h"
#include "editor_style.h"
#include "effects_panel.h"
#include "inspector_panel.h"
#include "inspector_widgets.h"
#include "main_window.h"
#include "media_artwork.h"
#include "player_overlay.h"
#include "presets_panel.h"
#include "program_level.h"
#include "project_launcher.h"
#include "project_panel.h"
#include "source_monitor_panel.h"
#include "test_footage.h"
#include "timeline_widget.h"
#include "toolbar_widgets.h"
#include "viewer_widget.h"
#include "workspace.h"

#include <QAbstractItemView>
#include <QAccessible>
#include <QAction>
#include <QActionGroup>
#include <QApplication>
#include <QCheckBox>
#include <QClipboard>
#include <QComboBox>
#include <QCoreApplication>
#include <QDialog>
#include <QDialogButtonBox>
#include <QDir>
#include <QDoubleSpinBox>
#include <QElapsedTimer>
#include <QFile>
#include <QFileInfo>
#include <QFormLayout>
#include <QFrame>
#include <QGroupBox>
#include <QGuiApplication>
#include <QHBoxLayout>
#include <QImage>
#include <QInputDialog>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMenu>
#include <QMenuBar>
#include <QMimeData>
#include <QPaintDevice>
#include <QPaintEngine>
#include <QPainter>
#include <QPixmap>
#include <QPlainTextEdit>
#include <QProcess>
#include <QProgressBar>
#include <QPushButton>
#include <QRadioButton>
#include <QRegularExpression>
#include <QScopeGuard>
#include <QScreen>
#include <QScrollArea>
#include <QScrollBar>
#include <QSet>
#include <QSettings>
#include <QSignalBlocker>
#include <QSignalSpy>
#include <QStandardPaths>
#include <QStatusBar>
#include <QStyleOptionViewItem>
#include <QTabWidget>
#include <QTemporaryDir>
#include <QTest>
#include <QTextBlock>
#include <QTextCursor>
#include <QTextEdit>
#include <QTextFrame>
#include <QTextLayout>
#include <QToolBar>
#include <QToolButton>
#include <QToolTip>
#include <QWheelEvent>
#include <QWidget>

#include <DockAreaTabBar.h>
#include <DockAreaTitleBar.h>
#include <DockAreaWidget.h>
#include <DockManager.h>
#include <DockSplitter.h>
#include <DockWidget.h>
#include <DockWidgetTab.h>
#include <ElidingLabel.h>

#include <algorithm>
#include <climits>
#include <cmath>
#include <functional>
#include <vector>

namespace kerf {

using namespace test;

class ToolbarTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    // The toolbar's format bank: a latching key per preset, the shape of
    // each drawn on it, named for assistive technology; ←→ walk the bank
    // and Space or Return chooses; a custom size latches none.
    void toolbarFormatBankLatchesThePreset() {
        FormatBank bank;
        const QList<CanvasPreset> presets = canvasPresets();
        QCOMPARE(bank.count(), int(presets.size()));
        const QString times = QStringLiteral(" × ");
        QCOMPARE(bank.key(1)->accessibleName(), QStringLiteral("Format 9:16 · 1080") + times + QStringLiteral("1920"));
        for (int i = 0; i < bank.count(); ++i) {
            FormatKey *key = bank.key(i);
            const QSize size(presets.at(i).width, presets.at(i).height);
            QCOMPARE(key->format(), size);
            QCOMPARE(key->accessibleName(), QStringLiteral("Format %1").arg(canvasSizeLabel(size)));
            QCOMPARE(key->toolTip(), canvasSizeLabel(size));
            QVERIFY(key->text().isEmpty());
            QCOMPARE(key->focusPolicy(), Qt::TabFocus);
            QCOMPARE(key->size(), QSize(FormatKey::kWidth, FormatKey::kHeight));
            // Each shape is the format's own, inside its key.
            const QRectF shape = key->shapeRect();
            QVERIFY(QRectF(key->rect()).contains(shape));
            const qreal aspect = qreal(size.width()) / size.height();
            QVERIFY2(std::abs(shape.width() / shape.height() - aspect) < 0.2, qPrintable(QString::number(shape.width() / shape.height())));
            QAccessibleInterface *access = QAccessible::queryAccessibleInterface(key);
            QVERIFY(access);
            QCOMPARE(access->role(), QAccessible::RadioButton);
        }
        // Latched: the preset's key alone; a custom size, none, and the
        // bank's tooltip names it.
        QCOMPARE(bank.currentIndex(), 0);
        bank.setCanvasSize(QSize(1080, 1920));
        QCOMPARE(bank.currentIndex(), 1);
        QVERIFY(QAccessible::queryAccessibleInterface(bank.key(1))->state().checked);
        QVERIFY(!bank.key(0)->isChecked());
        bank.setCanvasSize(QSize(1280, 720));
        QCOMPARE(bank.currentIndex(), -1);
        for (int i = 0; i < bank.count(); ++i) QVERIFY(!bank.key(i)->isChecked());
        QCOMPARE(bank.toolTip(), QStringLiteral("Custom · 1280") + times + QStringLiteral("720"));
        bank.setCanvasSize(QSize(1920, 1080));
        QCOMPARE(bank.currentIndex(), 0);

        // The keys sit in the bank's well, after the engraved label, 2px
        // apart and 2px in.
        bank.show();
        QVERIFY(QTest::qWaitForWindowExposed(&bank));
        bank.activateWindow();
        QVERIFY(QTest::qWaitForWindowActive(&bank));
        const QRectF well = bank.bankRect();
        QVERIFY(well.left() > 20);
        QCOMPARE(int(well.height()), FormatBank::kHeight);
        for (int i = 0; i < bank.count(); ++i) {
            QVERIFY(well.contains(QRectF(bank.key(i)->geometry())));
            QCOMPARE(bank.key(i)->y(), 2);
            if (i > 0) QCOMPARE(bank.key(i)->x() - bank.key(i - 1)->geometry().right() - 1, 2);
        }
        QCOMPARE(bank.key(0)->x() - int(well.left()), 2);

        // A click latches the key and asks once; the latched key asks nothing.
        QSignalSpy chosen(&bank, &FormatBank::formatChosen);
        QTest::mouseClick(bank.key(2), Qt::LeftButton);
        QCOMPARE(chosen.size(), 1);
        QCOMPARE(chosen.at(0).at(0).toSize(), QSize(1080, 1080));
        QCOMPARE(bank.currentIndex(), 2);
        // A click takes no keyboard focus.
        QVERIFY(!bank.key(2)->hasFocus());
        QTest::mouseClick(bank.key(2), Qt::LeftButton);
        QCOMPARE(chosen.size(), 1);
        // Tab rings a key; → moves along the bank, ringed, choosing nothing;
        // Return and Space choose.
        // (The window handed its first key focus as it opened, unringed.)
        QVERIFY(!bank.key(0)->showsFocusRing());
        bank.key(0)->clearFocus();
        bank.key(0)->setFocus(Qt::TabFocusReason);
        QVERIFY(bank.key(0)->showsFocusRing());
        QTest::keyClick(bank.key(0), Qt::Key_Right);
        QVERIFY(bank.key(1)->hasFocus());
        QVERIFY(bank.key(1)->showsFocusRing());
        QCOMPARE(chosen.size(), 1);
        QTest::keyClick(bank.key(1), Qt::Key_Return);
        QCOMPARE(chosen.size(), 2);
        QCOMPARE(chosen.at(1).at(0).toSize(), QSize(1080, 1920));
        QCOMPARE(bank.currentIndex(), 1);
        QTest::keyClick(bank.key(1), Qt::Key_Left);
        QVERIFY(bank.key(0)->hasFocus());
        QTest::keyClick(bank.key(0), Qt::Key_Left);
        QVERIFY(bank.key(0)->hasFocus());
        QTest::keyClick(bank.key(0), Qt::Key_Space);
        QCOMPARE(chosen.size(), 3);
        QCOMPARE(bank.currentIndex(), 0);
    }

    // The keys and their bank, flat on the ramp: the latched key is the on
    // fill with its LED lit in the accent, the others raised, the shapes in
    // the engraved ink.
    void toolbarFormatBankPaintsTheHardware() {
        {
            const EditorTheme &theme = editorTheme();
            FormatBank bank;
            bank.setCanvasSize(QSize(1080, 1920));
            bank.resize(bank.sizeHint());
            const QImage image = bank.grab().toImage();
            const qreal ratio = image.devicePixelRatio();
            const auto at = [&](const QWidget *key, QPointF point) {
                return image.pixelColor((QPointF(key->pos()) + point).toPoint() * ratio).rgb();
            };
            FormatKey *lit = bank.key(1);
            FormatKey *raised = bank.key(0);
            // The LED: a 10 × 2 bar at the top of the latched key's face.
            const QRgb led = at(lit, QPointF(lit->width() / 2.0, 3.5));
            QVERIFY2(nearColor(led, theme.keyLed.rgb(), 24), qPrintable(QColor(led).name()));
            QVERIFY(!nearColor(at(raised, QPointF(raised->width() / 2.0, 3.5)), theme.keyLed.rgb(), 40));
            // Held in: the latched face; raised: between the bevel's stops.
            const QPointF corner(3.5, lit->height() - 4.5);
            QVERIFY2(nearColor(at(lit, corner), theme.keyLatched.rgb(), 6), qPrintable(QColor(at(lit, corner)).name()));
            const QColor face = at(raised, corner);
            const auto between = [](int value, int a, int b) { return value >= std::min(a, b) - 3 && value <= std::max(a, b) + 3; };
            QVERIFY2(between(face.red(), theme.keyTop.red(), theme.keyBottom.red()) &&
                         between(face.green(), theme.keyTop.green(), theme.keyBottom.green()),
                     qPrintable(face.name()));
            // The shape's outline is the engraved ink.
            const QRectF shape = raised->shapeRect();
            QVERIFY2(nearColor(at(raised, QPointF(shape.left() + 0.25, shape.center().y())), theme.engravedInk.rgb(), 30),
                     qPrintable(QColor(at(raised, QPointF(shape.left() + 0.25, shape.center().y()))).name()));
            // The bank's well shows between the keys.
            const QRgb gap = at(raised, QPointF(raised->width() + 1, raised->height() - 3));
            QVERIFY2(nearColor(gap, theme.bankWell.rgb(), 6), qPrintable(QColor(gap).name()));
        }
    }

    // Export is one key in the accent: Export, a chip naming the quality,
    // and an arrow whose menu picks Master or Draft and holds Render
    // Preview; Export makes what the chip names.
    void toolbarExportSplitButton() {
        MainWindow window{QString(), QString()};
        window.resize(1366, 768);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ExportButton *button = window.m_exportButton;
        QAction *exportAction = window.m_commands->action(QStringLiteral("file.export"));
        QAction *render = window.m_commands->action(QStringLiteral("sequence.renderPreview"));
        QCOMPARE(button->height(), ExportButton::kHeight);
        QCOMPARE(button->mainPart()->accessibleName(), QStringLiteral("Export"));
        QCOMPARE(button->menuPart()->accessibleName(), QStringLiteral("Export options"));
        QVERIFY(button->mainPart()->accessibleDescription().startsWith(QStringLiteral("Master")));
        QCOMPARE(button->mainPart()->focusPolicy(), Qt::TabFocus);
        QCOMPARE(button->menuPart()->geometry().right(), button->width() - 1);
        QCOMPARE(button->menuPart()->width(), ExportButton::kArrowWidth);
        QVERIFY(QRectF(button->mainPart()->geometry()).contains(button->chipRect()));
        // The menu: Master and Draft, an exclusive checked pair, Master
        // first and chosen; then Render Preview, the command's own action.
        const QList<QAction *> items = button->menu()->actions();
        QCOMPARE(items.size(), 4);
        QCOMPARE(items.at(0), button->masterAction());
        QCOMPARE(items.at(1), button->draftAction());
        QVERIFY(items.at(2)->isSeparator());
        QCOMPARE(items.at(3), render);
        QVERIFY(button->masterAction()->isCheckable() && button->draftAction()->isCheckable());
        QVERIFY(button->masterAction()->actionGroup() && button->masterAction()->actionGroup()->isExclusive());
        QCOMPARE(button->masterAction()->actionGroup(), button->draftAction()->actionGroup());
        QVERIFY(button->draftAction()->text().contains(QStringLiteral("360p · 15 fps")));
        // Opening it names the size and rate Master makes.
        window.syncCanvasPreset(QSize(1080, 1920));
        button->showMenu();
        QVERIFY(QTest::qWaitForWindowExposed(button->menu()));
        QCOMPARE(button->masterAction()->text(), QStringLiteral("Master\t1080 × 1920 · %1 fps").arg(window.m_fps));
        // A press on the arrow while the menu is open closes it, and only
        // that: the menu asks Windows and Linux not to replay the press to
        // the arrow under it, which would open it again at once. (Offscreen
        // grabs no pointer, so replays nothing; the attribute is what asks.)
        {
            const auto pressAt = [&window, button](const QPoint &global) {
                QWindow *popup = button->menu()->windowHandle();
                QVERIFY(popup);
                QTest::mousePress(popup, Qt::LeftButton, Qt::NoModifier, popup->mapFromGlobal(global));
                QTest::mouseRelease(window.windowHandle(), Qt::LeftButton, Qt::NoModifier, window.windowHandle()->mapFromGlobal(global));
                QCoreApplication::processEvents();
            };
            pressAt(button->menuPart()->mapToGlobal(button->menuPart()->rect().center()));
            QVERIFY(!button->menu()->isVisible());
            QVERIFY(button->menu()->testAttribute(Qt::WA_NoMouseReplay));
            // Opened again, a press anywhere else still reaches what it lands on.
            button->showMenu();
            QVERIFY(QTest::qWaitForWindowExposed(button->menu()));
            QVERIFY(!button->menu()->testAttribute(Qt::WA_NoMouseReplay));
            pressAt(button->mainPart()->mapToGlobal(button->mainPart()->rect().center()));
            QVERIFY(!button->menu()->isVisible());
            QVERIFY(!button->menu()->testAttribute(Qt::WA_NoMouseReplay));
            button->showMenu();
            QVERIFY(QTest::qWaitForWindowExposed(button->menu()));
        }
        button->menu()->hide();
        // The main part follows file.export's enablement and its reason.
        QCOMPARE(button->mainPart()->isEnabled(), exportAction->isEnabled());
        QVERIFY(button->mainPart()->toolTip().startsWith(exportAction->toolTip()));
        exportAction->setEnabled(!exportAction->isEnabled());
        QCOMPARE(button->mainPart()->isEnabled(), exportAction->isEnabled());

        // Draft, chosen from the menu, is what the chip names and what
        // Export asks for.
        QSignalSpy changed(button, &ExportButton::qualityChanged);
        button->draftAction()->trigger();
        QCOMPARE(changed.size(), 1);
        QCOMPARE(button->quality(), QStringLiteral("preview"));
        QCOMPARE(button->qualityName(), QStringLiteral("Draft"));
        QVERIFY(button->mainPart()->accessibleDescription().startsWith(QStringLiteral("Draft")));
        window.setConnection(QStringLiteral("ready"));
        window.exportProject();
        QCOMPARE(window.m_lastCommand.value(QStringLiteral("quality")).toString(), QStringLiteral("preview"));
        button->masterAction()->trigger();
        window.exportProject();
        QCOMPARE(window.m_lastCommand.value(QStringLiteral("quality")).toString(), QStringLiteral("full"));

        // Its colours, in both appearances: the accent lit from above, the
        // chip and label in the ink on it; a disabled export a disabled bezel.
        {
            const EditorTheme &theme = editorTheme();
            exportAction->setEnabled(true);
            QImage image = button->grab().toImage();
            qreal ratio = image.devicePixelRatio();
            const QColor body = image.pixelColor(QPoint(4, button->height() / 2) * ratio);
            QVERIFY2(std::abs(body.hslHue() - theme.accent.hslHue()) <= 6, qPrintable(body.name()));
            QVERIFY(contrastRatio(theme.onAccent, theme.accentTop) >= 4.5);
            // The chip's edge is the ink, part-strength, on the accent.
            const QRectF chip = button->chipRect();
            const QColor edge = image.pixelColor(QPointF(chip.left() + 0.25, chip.center().y()).toPoint() * ratio);
            QVERIFY2(contrastRatio(edge, body) > 1.5, qPrintable(edge.name()));
            exportAction->setEnabled(false);
            image = button->grab().toImage();
            ratio = image.devicePixelRatio();
            QVERIFY(nearColor(image.pixelColor(QPoint(4, button->height() / 2) * ratio).rgb(), theme.buttonDisabled.rgb(), 3));
        }
    }

    // The title still centres between the toolbar's groups, and at
    // 1366 × 768 every control shows, none folded into the toolbar's »
    // extension.
    void toolbarFitsASmallWindow() {
        MainWindow window{QString(), QString()};
        window.applySnapshot(showcaseSnapshot(3, QStringLiteral("clean")));
        window.setProjectTitle(QStringLiteral("Harbor Teaser, the long cut for review"));
        window.resize(1366, 768);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QCoreApplication::processEvents();
        QToolBar *toolbar = window.m_toolbar;
        for (QAction *action : toolbar->actions()) {
            QWidget *item = toolbar->widgetForAction(action);
            if (!item || !action->isVisible()) continue;
            QVERIFY2(item->isVisible(), qPrintable(item->objectName()));
            QVERIFY2(QRect(QPoint(), toolbar->size()).contains(item->geometry()), qPrintable(item->objectName()));
        }
        if (auto *more = toolbar->findChild<QToolButton *>(QStringLiteral("qt_toolbar_ext_button"))) QVERIFY(!more->isVisible());
        QVERIFY(window.m_formatBank->isVisible() && window.m_exportButton->isVisible() && window.m_chatToggle->isVisible());
        QCOMPARE(window.m_chatToggle->mapTo(&window, QPoint(window.m_chatToggle->width(), 0)).x(), window.width() - 8);
        // The title keeps clear of the bank, the right-hand group's first control.
        QLabel *title = window.m_projectTitle;
        QVERIFY(!title->text().isEmpty());
        QVERIFY(title->geometry().left() > window.m_toolbarLeftEnd->geometry().right());
        QVERIFY2(title->geometry().right() < window.m_formatBank->geometry().left(),
                 qPrintable(QStringLiteral("%1 vs %2").arg(title->geometry().right()).arg(window.m_formatBank->geometry().left())));
        // Wide enough, it centres on the window.
        window.resize(1600, 900);
        QCoreApplication::processEvents();
        QVERIFY(std::abs(title->geometry().center().x() - toolbar->width() / 2) <= 1);

        // Renders for the eye: KERF_TOOLBAR_GALLERY=folder, both appearances.
        if (const QString folder = qEnvironmentVariable("KERF_TOOLBAR_GALLERY"); !folder.isEmpty()) {
            window.resize(1366, 768);
            window.setConnection(QStringLiteral("ready"));
            window.m_commands->action(QStringLiteral("file.export"))->setEnabled(true);
            {
                QCoreApplication::processEvents();
                const QString name = QStringLiteral("dark");
                toolbar->grab().save(QDir(folder).filePath(QStringLiteral("toolbar-%1.png").arg(name)));
                window.m_exportButton->showMenu();
                QVERIFY(QTest::qWaitForWindowExposed(window.m_exportButton->menu()));
                window.m_exportButton->menu()->grab().save(QDir(folder).filePath(QStringLiteral("export-menu-%1.png").arg(name)));
                window.m_exportButton->menu()->hide();
                window.syncCanvasPreset(QSize(1280, 720));
                toolbar->grab().save(QDir(folder).filePath(QStringLiteral("toolbar-custom-%1.png").arg(name)));
                window.syncCanvasPreset(QSize(1920, 1080));
            }
        }
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::ToolbarTest)

#include "toolbar_test.moc"
