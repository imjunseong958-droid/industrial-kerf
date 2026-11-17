// The whole editor against the approved minimal mockup (03-review-thread.html)
// at its 1600 x 1000 canvas: in 16:9 and 9:16, the base
// state and the Media sheet, fed the seed (comments,
// proposals, a check, loudness and a render queue) as kerfd sends it, through
// handleEvent. Each state's rects are checked against the brief's table, with no
// gaps, cards, radii or margins around the three regions.
//
// With KERF_SCREENSHOT_DIR set, every state is saved under the mockup's query
// state: <fmt>.png for ?fmt=, <fmt>-media.png for the sheet,
// and <fmt>-preview.png for ?preview=1.

#include "fixtures.h"
#include "editor_theme.h"
#include "test_registry.h"

#include "chat_panel.h"
#include "components.h"
#include "command_registry.h"
#include "design_tokens.h"
#include "main_window.h"
#include "test_footage.h"
#include "ui/editor_bus.h"
#include "ui/proposal_model.h"
#include "ui/wiring/chat_wiring.h"
#include "ui/agent_hud.h"
#include "ui/chat_messages.h"
#include "ui/proposal_card.h"
#include "ui/wiring/stage_wiring.h"
#include "ui/wiring/timeline_wiring.h"
#include "project_panel.h"
#include "timeline_widget.h"
#include "viewer_widget.h"
#include "ui/stage_shell.h"


#include <QAbstractButton>
#include <QAction>
#include <QApplication>
#include <QCoreApplication>
#include <QDir>
#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QScopeGuard>
#include <QSettings>
#include <QStatusBar>
#include <QTest>
#include <QToolBar>

#include <cmath>
#include <functional>

namespace kerf {

using namespace test;

namespace {

// A conversation of the mockup's: a request at 00:00:09:03, Kerf's answer
// with its steps and the proposal it staged (p7), and a follow-up.
QJsonObject seedConversation() {
    const auto item = [](const char *id, const char *kind, const char *at, const char *text,
                         std::initializer_list<std::pair<const char *, QJsonValue>> extra = {}) {
        QJsonObject out{{"id", id}, {"kind", kind}, {"text", text}, {"at", at}};
        for (const auto &[key, value] : extra) out.insert(key, value);
        return out;
    };
    return QJsonObject{
        {"type", "conversation"}, {"id", "c-seed"}, {"title", "The sea reads magenta here"},
        {"created", "2026-11-02T10:00:00Z"}, {"updated", "2026-11-03T10:05:00Z"},
        {"items", QJsonArray{
             item("i1", "you", "2026-11-02T10:00:00Z", "The sea reads magenta here", {{"timecode", 9.12}}),
             item("i2", "kerf", "2026-11-02T10:01:00Z", "Pulled the sea back toward cyan on **B003_C011**, from 00:00:09:03.",
                  {{"steps", QJsonArray{"look at 00:00:09:03", "edit fx B003_C011 add color.hue", "render preview"}}, {"seconds", 74.0},
                   {"proposal", "p7"}}),
             item("i3", "you", "2026-11-03T10:05:00Z", "Limit the music bus at the true-peak spike"),
             item("i4", "kerf", "2026-11-03T10:06:00Z", "The agent failed.", {{"error", true}, {"seconds", 3.0}}),
         }}};
}

// The mockup's eight QC rows (mockup-spec 12).
QJsonObject seedCheck() {
    const auto row = [](const char *code, const char *name, bool passed, const char *value) {
        return QJsonObject{{"code", code}, {"name", name}, {"passed", passed}, {"value", QString::fromUtf8(value)}};
    };
    return QJsonObject{{"type", "check"}, {"status", "done"}, {"valid", false}, {"findings", QJsonArray{}},
                       {"summary", QJsonObject{{"passed", 6}, {"total", 8}, {"blocking", 1}, {"rows", QJsonArray{
                           row("loudness.integrated", "Integrated loudness", true, "−23.1 LUFS"),
                           row("loudness.true_peak", "True peak", false, "−0.4 dBTP"),
                           row("loudness.range", "Loudness range", true, "6.2 LU"),
                           row("picture.out_of_range", "Gamut Rec.709", true, "legal"),
                           row("picture.luma_levels", "Luma levels", true, "Y′ 64–940"),
                           row("text.safe_area", "Title safe 90%", true, "0 outside"),
                           row("text.overlap", "Caption overlap", false, "1 at 00:00:17:08"),
                           row("picture.flash_black", "Flash and black", true, "0 · 0")}}}}};
}

QJsonObject seedLoudness() {
    QJsonArray series;
    for (int i = 0; i <= 180; ++i) {
        const double t = i / 10.0;
        const double s = -25.5 + 2.2 * std::sin(t / 3.1);
        series.append(QJsonArray{t, s + 1, t < 3 ? QJsonValue() : QJsonValue(s)});
    }
    return QJsonObject{{"type", "loudness"}, {"status", "ready"}, {"integrated", -23.1}, {"lra", 6.2},
                       {"truePeak", QJsonArray{-0.4, -1.3}}, {"shortTermMax", -14.9}, {"momentaryMax", -12.0},
                       {"series", series}, {"target", -23}, {"limit", -1}, {"duration", 18.0}};
}

QJsonObject seedQueue() {
    return QJsonObject{{"type", "render_queue"}, {"jobs", QJsonArray{
        QJsonObject{{"jobId", "j1"}, {"name", "atlas_draft"}, {"quality", "review"}, {"codec", "h264"}, {"status", "completed"},
                    {"progress", 1.0}, {"outputPath", "deliveries/atlas_draft.mp4"}},
        QJsonObject{{"jobId", "j2"}, {"name", "atlas_master"}, {"quality", "master"}, {"codec", "prores_hq"}, {"status", "running"},
                    {"progress", 0.42}}}}};
}

// The showcase cut in `canvas`'s shape, with the seed's proposals.
QJsonObject seedState(const QSize &canvas) {
    QJsonObject snapshot = showcaseSnapshot(22, QStringLiteral("clean"));
    QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
    timeline.insert(QStringLiteral("width"), canvas.width());
    timeline.insert(QStringLiteral("height"), canvas.height());
    snapshot.insert(QStringLiteral("timeline"), timeline);
    snapshot.insert(QStringLiteral("reviewRevision"), 41);
    return QJsonObject{{"type", "state"}, {"snapshot", snapshot}};
}

struct Format {
    const char *query; // the mockup's ?fmt=
    QSize canvas;
    QRect slot; // the viewer
    QRect picture;
    QRect conversation;
    QRect timeline;
};

// The layout at 1600 x 1000, in window coordinates (ui/stage_shell.h).
const Format kFormats[] = {
    {"169", QSize(1920, 1080), QRect(768, 40, 832, 420), QRect(832, 52, 704, 396), QRect(320, 40, 448, 960), QRect(768, 460, 832, 540)},
    {"916", QSize(1080, 1920), QRect(768, 40, 832, 614), QRect(1018, 52, 332, 590), QRect(320, 40, 448, 960), QRect(768, 654, 832, 346)},
};

QRect inWindow(QWidget *widget, QWidget *window) { return QRect(widget->mapTo(window, QPoint()), widget->size()); }

} // namespace

class MockupParityTest final : public QObject {
    Q_OBJECT

private slots:
    void init() {
        QSettings().remove(QStringLiteral("workspace"));
        QSettings().remove(QStringLiteral("viewer"));
    }
    void cleanup() {
        QSettings().remove(QStringLiteral("workspace"));
        QSettings().remove(QStringLiteral("viewer"));
    }

    void editorMatchesTheMockupInEveryState() {
        // Every kit motion lands at once, so each state is drawn settled.
        ui::setReducedMotionOverride(1);
        const auto motion = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        if (!shots.isEmpty()) QVERIFY(QDir().mkpath(shots));
        int states = 0;
        for (const Format &format : kFormats) {
            MainWindow window{QString(), QString()};
            // An agent that can take requests, as the mockup's composer shows.
            window.m_cloudAccess = QStringLiteral("ready");
            window.setConnection(QStringLiteral("ready"));
            window.handleEvent(QJsonObject{{"type", "collab"}, {"status", sharedStatus()}});
            window.handleEvent(seedState(format.canvas));
            window.m_chatWiring->openConversation(QStringLiteral("c-seed"));
            window.handleEvent(seedConversation());
            window.handleEvent(seedCheck());
            window.handleEvent(seedLoudness());
            window.handleEvent(seedQueue());
            fakeArtwork(window.m_artwork);
            window.resize(1600, 1000);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            window.m_errorBanner->hide();
            window.m_viewer->setFrame(testing::footageFrame(testing::Shot::HarborWide, format.canvas / 2), 0.0);
            QVERIFY(window.m_bus->proposals());
            QCOMPARE(window.m_chat->requests().size(), 2);
            const QString fmt = QString::fromLatin1(format.query);
            const auto save = [&window, &shots, &states](const QString &label) {
                for (int i = 0; i < 3; ++i) QCoreApplication::processEvents();
                ++states;
                if (shots.isEmpty()) return true;
                return window.grab().toImage().save(QDir(shots).filePath(label + QStringLiteral(".png")));
            };

            // The base state: the picture, the conversation and the timeline.
            QCoreApplication::processEvents();
            checkBase(window, format);
            checkNothingElseShows(window);
            // One painted hairline between the picture and the timeline, and
            // none beside the sidebar: its own colour sets it off.
            {
                const QImage shot = window.grab().toImage();
                const int chrome = tokens::color(tokens::chrome).red();
                QVERIFY(shot.pixelColor(format.conversation.left(), format.conversation.top() + 100).red() <= chrome);
                // Over the timeline under a wide picture, at its left beside a vertical one.
                const QPoint timelineSeam = format.timeline.top() > 40 ? format.timeline.topLeft() + QPoint(100, 0) : format.timeline.topLeft() + QPoint(0, 100);
                QVERIFY(shot.pixelColor(timelineSeam).red() > chrome);
            }
            QVERIFY(save(fmt));

            // The Media band, across the panel between the picture and the timeline.
            const struct {
                StageShell::Sheet sheet;
                const char *name;
                const char *command;
                QWidget *widget;
            } sheets[] = {{StageShell::Sheet::Media, "media", "window.project", window.m_projectPanel}};
            for (const auto &sheet : sheets) {
                window.triggerCommand(QString::fromLatin1(sheet.command));
                QCOMPARE(window.m_shell->sheet(), sheet.sheet);
                QVERIFY2(sheet.widget->isVisible(), sheet.name);
                // The timeline gives it its top: it lies over neither the
                // picture nor what is said.
                const QRect band = inWindow(sheet.widget, &window);
                QCOMPARE(band.size(), QSize(format.timeline.width(), tokens::layout::mediaBand));
                QCOMPARE(band.left(), format.timeline.left());
                QCOMPARE(band.top(), inWindow(window.m_viewer, &window).bottom() + 1);
                QCOMPARE(inWindow(window.m_timelinePanel, &window).top(), band.bottom() + 1);
                QCOMPARE(inWindow(window.m_chat, &window), format.conversation);
                QVERIFY(save(QStringLiteral("%1-%2").arg(fmt, QString::fromLatin1(sheet.name))));
                window.triggerCommand(QString::fromLatin1(sheet.command));
                QCOMPARE(window.m_shell->sheet(), StageShell::Sheet::None);
                // Closed, everything is where it was.
                checkBase(window, format);
            }

            // ?preview=1: the chip, 12 inside the picture's top left.
            window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "ready"}, {"proposalId", "p7"}});
            PreviewChip *chip = window.m_stageWiring->previewChip();
            QVERIFY(chip->isVisible());
            QCOMPARE(chip->text(), QStringLiteral("Previewing proposal v1"));
            // .ptxt: the 6 pt ink2 dot 16 in, the text 8 past it in ink3.
            auto *dot = chip->findChild<ui::Dot *>(QStringLiteral("previewChipDot"));
            QVERIFY(dot);
            QCOMPARE(dot->color(), tokens::color(tokens::ink2));
            QCOMPARE(dot->geometry(), QRect(16, 9, 6, 6));
            auto *chipText = chip->findChild<QLabel *>(QStringLiteral("previewChipText"));
            QCOMPARE(chipText->geometry().left(), 30);
            QVERIFY(chipText->styleSheet().contains(tokens::color(tokens::ink3).name()));
            QCOMPARE(chip->height(), 24);
            QCOMPARE(inWindow(chip, &window).topLeft(), format.picture.topLeft() + QPoint(12, 12));
            QVERIFY(save(QStringLiteral("%1-preview").arg(fmt)));
            window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "exited"}});
            QVERIFY(!chip->isVisible());

            // Rows and proposal cards fill the panel's full width, at every format.
            checkConversationRows(window);
            window.close();
        }
        QCOMPARE(states, 2 * (1 + 1 + 1));
    }

    // KERF_SCREENSHOT_DIR=<dir> saves the window as its panes open and close,
    // frame by frame, to look at: motion can only be judged where it runs.
    void paneMovesForTheGallery() {
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        if (shots.isEmpty()) QSKIP("KERF_SCREENSHOT_DIR is not set");
        QVERIFY(QDir().mkpath(shots));
        MainWindow window{QString(), QString()};
        window.m_cloudAccess = QStringLiteral("ready");
        window.setConnection(QStringLiteral("ready"));
        window.handleEvent(seedState(QSize(1920, 1080)));
        window.m_chatWiring->openConversation(QStringLiteral("c-seed"));
        window.handleEvent(seedConversation());
        fakeArtwork(window.m_artwork);
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.m_errorBanner->hide();
        window.m_viewer->setFrame(testing::footageFrame(testing::Shot::HarborWide, QSize(960, 540)), 0.0);
        QCoreApplication::processEvents();
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        // Each move, from the frame it starts on to the one it rests on.
        const auto film = [&window, &shots](const QString &name, const std::function<void()> &start) {
            start();
            int frame = 0;
            int waited = 0;
            for (const int at : {0, 50, 110, 190, 300}) {
                QTest::qWait(at - waited);
                waited = at;
                window.grab().save(QDir(shots).filePath(QStringLiteral("move-%1-%2.png").arg(name).arg(frame++)));
            }
            QTRY_VERIFY_WITH_TIMEOUT(!window.m_shell->isMoving(), 3000);
            window.grab().save(QDir(shots).filePath(QStringLiteral("move-%1-%2.png").arg(name).arg(frame)));
        };
        film(QStringLiteral("panel-close"), [&window] { window.m_panelButton->click(); });
        film(QStringLiteral("panel-open"), [&window] { window.m_panelButton->click(); });
        film(QStringLiteral("media-open"), [&window] { window.triggerCommand(QStringLiteral("window.project")); });
        film(QStringLiteral("media-close"), [&window] { window.triggerCommand(QStringLiteral("window.project")); });
        film(QStringLiteral("sidebar-close"), [&window] { window.triggerCommand(QStringLiteral("view.sidebar")); });
        film(QStringLiteral("sidebar-open"), [&window] { window.triggerCommand(QStringLiteral("view.sidebar")); });
        window.close();
    }

    // KERF_SCREENSHOT_DIR=<dir> saves a conversation in each face Settings
    // offers, in Latin and in Hangul, and Settings' own pages, to look at.
    void facesForTheGallery() {
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        if (shots.isEmpty()) QSKIP("KERF_SCREENSHOT_DIR is not set");
        QVERIFY(QDir().mkpath(shots));
        const FaceChoice was = loadFaceChoice();
        const auto restore = qScopeGuard([was] {
            setEditorFaces(was);
            QSettings().remove(QStringLiteral("appearance"));
        });
        MainWindow window{QString(), QString()};
        window.m_cloudAccess = QStringLiteral("ready");
        window.setConnection(QStringLiteral("ready"));
        window.handleEvent(seedState(QSize(1920, 1080)));
        window.m_chatWiring->openConversation(QStringLiteral("c-faces"));
        const auto item = [](const char *id, const char *kind, const QString &text) {
            return QJsonObject{{"id", id}, {"kind", kind}, {"text", text}, {"at", "2026-11-02T10:00:00Z"}};
        };
        QJsonObject answer = item("i2", "kerf",
                                  QStringLiteral("가장 좋은 테이크 세 개로 **30초 컷**을 만들었어요. 와이드 숏으로 열고, 인터뷰의 쉬는 구간을 줄였습니다.\n\n"
                                                 "- Opened on the **wide shot** (0:00 to 0:04)\n- Tightened the pauses between takes\n"
                                                 "- 마지막은 클로즈업으로 끝납니다\n\nWant captions next? `title size=0.22` is the new value."));
        answer.insert(QStringLiteral("steps"), QJsonArray{QStringLiteral("look · done"), QStringLiteral("edit · done")});
        answer.insert(QStringLiteral("seconds"), 42.0);
        window.handleEvent(QJsonObject{
            {"type", "conversation"}, {"id", "c-faces"}, {"title", QStringLiteral("가장 좋은 테이크로 30초 컷")},
            {"created", "2026-11-02T10:00:00Z"}, {"updated", "2026-11-03T10:05:00Z"},
            {"items", QJsonArray{item("i1", "you", QStringLiteral("가장 좋은 테이크로 30초 컷을 만들어 줘. Keep the wide shot at the start.")), answer}}});
        fakeArtwork(window.m_artwork);
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.m_errorBanner->hide();
        window.setPanelOpen(false);
        // How full the context is, and a mode that lets edits through.
        window.m_chat->setContextUse(QStringLiteral("c-faces"), 131'000, 192'000, false);
        window.m_chat->setPermissionMode(PermissionMode::AcceptEdits);
        const auto save = [&window, &shots](const QString &name) {
            for (int i = 0; i < 4; ++i) QCoreApplication::processEvents();
            return window.grab().save(QDir(shots).filePath(name + QStringLiteral(".png")));
        };
        const struct {
            const char *name;
            FaceChoice choice;
        } faces[] = {{"faces-inter", {QStringLiteral("inter"), QStringLiteral("same"), QStringLiteral("default")}},
                     {"faces-pretendard", {QStringLiteral("pretendard"), QStringLiteral("same"), QStringLiteral("default")}},
                     {"faces-serif", {QStringLiteral("inter"), QStringLiteral("serif"), QStringLiteral("default")}},
                     {"faces-serif-large", {QStringLiteral("pretendard"), QStringLiteral("serif"), QStringLiteral("large")}}};
        for (const auto &face : faces) {
            setEditorFaces(face.choice);
            QVERIFY(save(QString::fromLatin1(face.name)));
        }
        setEditorFaces(FaceChoice{});
        for (const char *page : {"account", "ai", "appearance", "general", "advanced"}) {
            window.showSettings(QString::fromLatin1(page));
            QVERIFY(save(QStringLiteral("settings-%1").arg(QString::fromLatin1(page))));
        }
        window.close();
    }

    // The timeline at the design's size: its strip, heads, default heights, and a
    // scale that fits the sequence to the lanes.
    void timelineMatchesTheMockup() {
        for (const Format &format : kFormats) {
            MainWindow window{QString(), QString()};
            window.setConnection(QStringLiteral("ready"));
            window.handleEvent(seedState(format.canvas));
            window.resize(1600, 1000);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            QCoreApplication::processEvents();
            TimelineToolStrip *strip = window.m_timelineWiring->tools();
            QCOMPARE(strip->height(), 28);
            QCOMPARE(inWindow(strip, &window), QRect(format.timeline.topLeft(), QSize(format.timeline.width(), 28)));
            QCOMPARE(window.m_timeline->trackHeaderRect(0).width(), 140);
            QVERIFY(window.m_timeline->isFitToWidth());
            QCOMPARE(window.m_timeline->wideSequence(), format.canvas.width() > format.canvas.height());
            QCOMPARE(window.m_timelineScroll->viewport()->height(), format.timeline.height() - 28);
            window.close();
        }
    }

    // The real launch: shown and activated while connecting (the timeline is
    // disabled, so Qt hands the keys to the next control), then state and
    // ready in the bridge's order. The timeline's keys must work.
    void theKeysStartOnTheTimeline() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        window.activateWindow();
        QVERIFY(QTest::qWaitForWindowActive(&window));
        QCoreApplication::processEvents();
        QVERIFY(!window.m_timeline->isEnabled());
        window.handleEvent(seedState(QSize(1920, 1080)));
        window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "ready"}});
        QVERIFY(window.m_timeline->hasFocus());
        // Space plays, C takes the razor, I sets the mark.
        window.m_lastCommand = {};
        QTest::keyClick(QApplication::focusWidget(), Qt::Key_Space);
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("play"));
        QTest::keyClick(QApplication::focusWidget(), Qt::Key_C);
        QCOMPARE(window.m_timeline->tool(), TimelineTool::Razor);
        QVERIFY(!window.m_stageWiring->hasIn());
        QTest::keyClick(QApplication::focusWidget(), Qt::Key_I);
        QVERIFY(window.m_stageWiring->hasIn());
        // A reconnection takes the same path.
        window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "disconnected"}, {"message", "gone"}});
        QVERIFY(!window.m_timeline->hasFocus());
        window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "ready"}});
        QVERIFY(window.m_timeline->hasFocus());
        window.close();
    }

    // Coming ready leaves the keys with someone typing, and with a sheet in use.
    void comingReadyLeavesTypingAndSheetsAlone() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.handleEvent(seedState(QSize(1920, 1080)));
        window.show();
        window.activateWindow();
        QVERIFY(QTest::qWaitForWindowActive(&window));
        QCoreApplication::processEvents();
        window.triggerCommand(QStringLiteral("window.project"));
        window.m_projectPanel->focusSearch();
        QLineEdit *search = window.m_projectPanel->m_search;
        QVERIFY(search->hasFocus());
        window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "ready"}});
        QVERIFY(search->hasFocus());
        // Not a text field, but in the open sheet.
        QAbstractButton *inSheet = nullptr;
        for (QAbstractButton *button : window.m_projectPanel->findChildren<QAbstractButton *>())
            if (button->isVisible() && button->isEnabled()) inSheet = button;
        QVERIFY(inSheet);
        inSheet->setFocus();
        window.settleFocus();
        QCOMPARE(window.focusWidget(), inSheet);
        // The sheet closed, they go back to the timeline.
        window.triggerCommand(QStringLiteral("window.project"));
        QVERIFY(window.m_timeline->hasFocus());
        window.close();
    }

    // The keys start on the timeline the first time the window is active;
    // coming back to the window later keeps the button a keyboard user
    // tabbed to (settleFocus runs on the first activation, not on each).
    void comingBackKeepsTheKeyboardsPlace() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.handleEvent(seedState(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        window.activateWindow();
        QVERIFY(QTest::qWaitForWindowActive(&window));
        QCoreApplication::processEvents();
        QVERIFY(window.m_focusSettled);
        QVERIFY(window.m_timeline->hasFocus());
        QAbstractButton *button = nullptr;
        for (QAbstractButton *candidate : window.findChildren<QAbstractButton *>())
            if (candidate->isVisible() && candidate->isEnabled() && (candidate->focusPolicy() & Qt::TabFocus)) {
                button = candidate;
                break;
            }
        QVERIFY(button);
        button->setFocus(Qt::TabFocusReason);
        QCOMPARE(QApplication::focusWidget(), button);
        // Another app came forward and the window is active again.
        QEvent activation(QEvent::ActivationChange);
        QCoreApplication::sendEvent(&window, &activation);
        QVERIFY(window.isActiveWindow());
        QCOMPARE(QApplication::focusWidget(), button);
        window.close();
    }

private:
    void checkBase(MainWindow &window, const Format &format) {
        QCOMPARE(window.m_toolbar->height(), 40);
        QCOMPARE(inWindow(window.m_viewer, &window), format.slot);
        // With no margin: the frame fills its region's height, or its width too.
        QCOMPARE(window.m_viewer->videoRect(), QRectF(format.picture.translated(-format.slot.topLeft())));
        QCOMPARE(inWindow(window.m_chat, &window), format.conversation);
        QCOMPARE(inWindow(window.m_timelinePanel, &window), format.timeline);
        QVERIFY(window.m_viewer->isVisible());
        QVERIFY(window.m_chat->isVisible());
        QVERIFY(window.m_timelinePanel->isVisible());
    }

    void checkNothingElseShows(MainWindow &window) {
        QVERIFY(!window.m_projectPanel->isVisible());
        QVERIFY(!window.findChild<QStatusBar *>());
        QVERIFY(window.m_stageWiring->previewChip()->isHidden());
        QVERIFY(window.m_stageWiring->hud()->isHidden());
        // Safe zones are off by default.
        QVERIFY(!window.m_viewer->safeZonesShown());
    }

    void checkConversationRows(MainWindow &window) {
        ChatPanel *chat = window.m_chat;
        chat->m_content->layout()->activate();
        QCoreApplication::processEvents();
        const int width = chat->scroll()->viewport()->width();
        QCOMPARE(chat->m_content->width(), width);
        // Within the column it keeps to read in, when it has more room than that.
        const int inset = std::max(0, (chat->width() - tokens::layout::conversationColumn) / 2);
        const int right = width - ChatPanel::kSidePadding - inset;
        const QList<QWidget *> messages = chat->messages();
        QVERIFY(!messages.isEmpty());
        for (const QWidget *message : messages) {
            if (!message->isVisibleTo(chat)) continue;
            // Every message stands as wide as the column; a request's bubble keeps to its right.
            QCOMPARE(message->mapTo(chat->m_content, QPoint(message->width(), 0)).x(), right);
            if (const auto *you = qobject_cast<const YouMessage *>(message))
                QCOMPARE(you->bubbleRect().right() + 1, you->width());
        }
        for (const ProposalCard *card : chat->cards()) {
            if (!card->isVisibleTo(chat)) continue;
            QCOMPARE(card->mapTo(chat->m_content, QPoint(card->width(), 0)).x(), right);
        }
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::MockupParityTest)

#include "mockup_parity_test.moc"
