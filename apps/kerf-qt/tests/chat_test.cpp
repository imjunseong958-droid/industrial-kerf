// Tests for the conversation: the conversations kept beside the project
// (ui/conversation_model.h) as the bridge sends them, the panel that reads
// one top to bottom (chat_panel.h, ui/chat_messages.h), and the wiring that
// sends what is asked (agent, conversations) and feeds the sidebar.

#include "fixtures.h"
#include "test_registry.h"

#include "chat_panel.h"
#include "components.h"
#include "composer.h"
#include "design_tokens.h"
#include "editor_theme.h"
#include "main_window.h"
#include "project_sidebar.h"
#include "ui/chat_messages.h"
#include "ui/chat_parts.h"
#include "ui/conversation_model.h"
#include "ui/editor_bus.h"
#include "ui/wiring/chat_wiring.h"

#include <QAbstractButton>
#include <QSettings>
#include <QScopeGuard>
#include <QFontDatabase>
#include <QApplication>
#include <QCoreApplication>
#include <QJsonArray>
#include <QJsonObject>
#include <QLabel>
#include <QMenu>
#include <QSignalSpy>
#include <QTest>
#include <QTextEdit>
#include <QTimeZone>

#include <cmath>
#include <initializer_list>

namespace kerf {

using namespace test;

namespace {

QJsonObject item(const char *id, const char *kind, const char *text, std::initializer_list<std::pair<const char *, QJsonValue>> extra = {}) {
    QJsonObject out{{"id", id}, {"kind", kind}, {"text", text}, {"at", "2026-11-16T09:00:00.123456Z"}};
    for (const auto &[key, value] : extra) out.insert(key, value);
    return out;
}

QJsonObject conversation(const char *id, const char *title, const QJsonArray &items, bool archived = false) {
    return QJsonObject{{"type", "conversation"}, {"id", id}, {"title", title}, {"created", "2026-11-16T09:00:00.000000Z"},
                       {"updated", "2026-11-16T09:05:00.000000Z"}, {"archived", archived}, {"items", items}};
}

QJsonObject listed(std::initializer_list<QJsonObject> items) {
    QJsonArray out;
    for (const QJsonObject &one : items) out << one;
    return QJsonObject{{"type", "conversations"}, {"items", out}};
}

QJsonObject summary(const char *id, const char *title, int count, bool archived = false, const char *updated = "2026-11-16T09:05:00Z") {
    return QJsonObject{{"id", id}, {"title", title}, {"created", "2026-11-16T09:00:00Z"}, {"updated", updated}, {"archived", archived},
                       {"count", count}};
}

void settle() {
    for (int i = 0; i < 4; ++i) QCoreApplication::processEvents();
}

} // namespace

class ChatTest final : public QObject {
    Q_OBJECT

    // A window whose bus records what it sends, connected.
    struct Window {
        MainWindow window{QString(), QString()};
        QList<QJsonObject> sent;
        Window() {
            window.m_bus->setSender([this](const QJsonObject &command) {
                sent << command;
                return true;
            });
            window.setConnection(QStringLiteral("ready"));
            window.resize(1600, 1000);
        }
        QList<QJsonObject> of(const char *type) const {
            QList<QJsonObject> out;
            for (const QJsonObject &command : sent)
                if (command.value("type").toString() == QLatin1String(type)) out << command;
            return out;
        }
        ChatPanel *chat() const { return window.m_chat; }
        ConversationModel *conversations() const { return window.m_chatWiring->conversations(); }
    };


private slots:
    // The list the sidebar shows and the one conversation open, as the bridge
    // reports them; a request sent with none open adopts the one it started.
    void theModelReadsTheBridge() {
        ConversationModel model;
        QSignalSpy list(&model, &ConversationModel::listChanged);
        QSignalSpy current(&model, &ConversationModel::currentChanged);
        QVERIFY(!model.listed());
        QVERIFY(model.applyEvent(QStringLiteral("conversations"),
                                 listed({summary("c-1", "Cut the harbor", 4), summary("c-2", "Captions", 2, true)})));
        QVERIFY(model.listed());
        QCOMPARE(list.count(), 1);
        QCOMPARE(model.list().size(), 2);
        QCOMPARE(model.summary(QStringLiteral("c-2"))->archived, true);
        QCOMPARE(model.list().first().updated, QDateTime(QDate(2026, 11, 16), QTime(9, 5), QTimeZone::utc()));
        // Another conversation's whole is not the open one's.
        QVERIFY(model.applyEvent(QStringLiteral("conversation"), conversation("c-1", "Cut the harbor", {item("i1", "you", "Cut it")})));
        QCOMPARE(current.count(), 0);
        // Chosen, it shows its title at once and its messages when they come.
        model.choose(QStringLiteral("c-1"));
        QCOMPARE(current.count(), 1);
        QCOMPARE(model.current().summary.title, QStringLiteral("Cut the harbor"));
        QVERIFY(model.current().items.isEmpty());
        model.applyEvent(QStringLiteral("conversation"),
                         conversation("c-1", "Cut the harbor",
                                      {item("i1", "you", "Cut it", {{"timecode", 9.12}}),
                                       item("i2", "kerf", "Cut.", {{"steps", QJsonArray{"look", "edit trim"}}, {"seconds", 72.4}, {"proposal", "p1"}}),
                                       item("i3", "you", "Again"), item("i4", "kerf", "The agent failed.", {{"error", true}}),
                                       item("i5", "kerf", "Stopped before finishing.", {{"stopped", true}})}));
        QCOMPARE(current.count(), 2);
        const QList<ConversationModel::Item> &items = model.current().items;
        QCOMPARE(items.size(), 5);
        QVERIFY(items.at(0).you && items.at(0).hasTimecode());
        QCOMPARE(items.at(0).timecode, 9.12);
        QVERIFY(items.at(0).at.isValid());
        QCOMPARE(items.at(1).steps, (QStringList{"look", "edit trim"}));
        QCOMPARE(items.at(1).seconds, 72.4);
        QCOMPARE(items.at(1).proposal, QStringLiteral("p1"));
        QVERIFY(!items.at(2).hasTimecode());
        QVERIFY(items.at(3).error && items.at(4).stopped);
        // A request with none open: the next conversation reported is its.
        model.choose(QString());
        QVERIFY(model.currentId().isEmpty());
        model.expectNew();
        model.applyEvent(QStringLiteral("conversation"), conversation("c-3", "Make a teaser", {item("i1", "you", "Make a teaser")}));
        QCOMPARE(model.currentId(), QStringLiteral("c-3"));
        QVERIFY(!model.expectsNew());
        // Deleted while open: a new conversation in its place.
        model.applyEvent(QStringLiteral("conversations"), listed({summary("c-1", "Cut the harbor", 4)}));
        QVERIFY(model.currentId().isEmpty());
    }

    // What the person asked: a bubble at the right, at most 82% of the
    // column, its timecode a chip over it that seeks.
    void aRequestIsABubbleAtTheRight() {
        YouMessage you(QStringLiteral("Tighten the interview at the harbor"), 9.12, 25.0);
        you.resize(520, you.heightForWidth(520));
        you.show();
        QVERIFY(you.hasHeightForWidth());
        const QRect bubble = you.bubbleRect();
        QCOMPARE(bubble.right() + 1, you.width());
        QVERIFY(bubble.width() <= int(520 * YouMessage::kShare));
        QVERIFY(you.chip());
        QCOMPARE(you.chip()->geometry().right() + 1, you.width());
        QVERIFY(bubble.top() >= you.chip()->geometry().bottom());
        QSignalSpy seek(&you, &YouMessage::seekRequested);
        static_cast<QAbstractButton *>(you.chip())->click();
        QCOMPARE(seek.count(), 1);
        QCOMPARE(seek.first().first().toDouble(), 9.12);
        // A long request wraps within the share; a short one keeps its words' width.
        YouMessage longer(QString(40, QLatin1Char('w')) + QStringLiteral(" ") + QString(300, QLatin1Char('a')), std::nan(""), 25.0);
        longer.resize(520, longer.heightForWidth(520));
        QVERIFY(!longer.chip());
        QVERIFY(longer.bubbleRect().width() <= int(520 * YouMessage::kShare));
        QVERIFY(longer.bubbleRect().width() > bubble.width());
        QVERIFY(longer.heightForWidth(520) > you.heightForWidth(520) - YouMessage::kPadY);
    }

    // Kerf at work: "Working" with the time so far and the step it is on, the
    // last steps on a hairline; done, folded to "Worked for …", and unfolding
    // to every step.
    void theWorkLogSaysWhatKerfDid() {
        QCOMPARE(workClock(42.7), QStringLiteral("0:42"));
        QCOMPARE(workClock(725), QStringLiteral("12:05"));
        QCOMPARE(workLength(12.2), QStringLiteral("12s"));
        QCOMPARE(workLength(72.4), QStringLiteral("1m 12s"));
        QCOMPARE(workLength(3840), QStringLiteral("1h 4m"));
        WorkLog log;
        log.resize(400, 20);
        QVERIFY(log.isHidden());
        log.start();
        QVERIFY(log.isLive() && log.isOpen());
        QVERIFY(log.headline().startsWith(QStringLiteral("Working · 0:0")));
        QCOMPARE(log.heightForWidth(400), WorkLog::kRow);
        for (const char *step : {"look at 00:00:09:03", "edit trim v1b out -12f", "render preview", "make shot-2"}) log.addStep(QString::fromLatin1(step));
        log.addStep(QStringLiteral("make shot-2")); // the same again is one step
        QCOMPARE(log.steps().size(), 4);
        // Live, the latest three under the line.
        QCOMPARE(log.heightForWidth(400), WorkLog::kRow + 4 + WorkLog::kLiveSteps * WorkLog::kRow);
        QVERIFY(log.toggle()->isHidden());
        log.setStatus(QStringLiteral("Rendering preview 40%"));
        QCOMPARE(log.status(), QStringLiteral("Rendering preview 40%"));
        log.finish(72.4);
        QVERIFY(!log.isLive() && !log.isOpen());
        QCOMPARE(log.headline(), QStringLiteral("Worked for 1m 12s · 4 steps"));
        QCOMPARE(log.toggle()->text(), log.headline());
        QVERIFY(!log.toggle()->isHidden());
        QCOMPARE(log.heightForWidth(400), WorkLog::kRow);
        log.toggle()->click();
        QVERIFY(log.isOpen());
        QCOMPARE(log.heightForWidth(400), WorkLog::kRow + 4 + 4 * WorkLog::kRow);
        // With no steps, nothing to unfold.
        WorkLog quick;
        quick.finish(3.0);
        QCOMPARE(quick.headline(), QStringLiteral("Worked for 3s"));
        QVERIFY(!quick.toggle()->isEnabled());
    }

    // A turn reads top to bottom: the request at once, Kerf at work under it
    // with its steps and its words as they stream; kept by the bridge, the
    // conversation's own messages take the live ones' place.
    void aTurnReadsTopToBottom() {
        Window w;
        w.window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&w.window));
        ChatPanel *chat = w.chat();
        QVERIFY(chat->welcome()->isVisible());
        QCOMPARE(chat->title()->property("fullTitle").toString(), QStringLiteral("New conversation"));
        QVERIFY(!chat->examples().isEmpty());

        chat->composerWidget()->fill(QStringLiteral("Make a 30-second cut from the harbor takes"));
        QVERIFY(w.window.m_chatWiring->post());
        const QList<QJsonObject> agent = w.of("agent");
        QCOMPARE(agent.size(), 1);
        QCOMPARE(agent.first().value("text").toString(), QStringLiteral("Make a 30-second cut from the harbor takes"));
        QVERIFY(!agent.first().contains("conversation"));
        QVERIFY(!agent.first().contains("comment"));
        QVERIFY(w.conversations()->expectsNew());
        QVERIFY(chat->composerWidget()->text().isEmpty());
        settle();
        QVERIFY(!chat->welcome()->isVisible());
        QCOMPARE(chat->requests().size(), 1);
        QVERIFY(chat->turnLive());
        KerfMessage *live = chat->liveMessage();
        QVERIFY(live && live->work()->isLive());
        QVERIFY(live->isDrafting());
        QCOMPARE(chat->title()->property("fullTitle").toString(), QStringLiteral("Make a 30-second cut from the harbor takes"));

        // The bridge keeps the request and says which conversation it started.
        w.window.handleEvent(conversation("c-9", "Make a 30-second cut", {item("i1", "you", "Make a 30-second cut from the harbor takes")}));
        settle();
        QCOMPARE(w.conversations()->currentId(), QStringLiteral("c-9"));
        QCOMPARE(chat->requests().size(), 1);
        QCOMPARE(chat->liveMessage(), live);

        w.window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", true}});
        w.window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "tool"}, {"text", "look at the harbor takes"}});
        w.window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "status"}, {"text", "Rendering preview 40%"}});
        w.window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "delta"}, {"text", "Cut the harbor "}});
        w.window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "delta"}, {"text", "takes to **30 s**."}});
        settle();
        QCOMPARE(live->work()->steps(), QStringList{QStringLiteral("look at the harbor takes")});
        QCOMPARE(live->work()->status(), QStringLiteral("Rendering preview 40%"));
        QVERIFY(!live->isDrafting());
        QCOMPARE(live->text(), QStringLiteral("Cut the harbor takes to **30 s**."));
        QVERIFY(live->body()->text().contains(QStringLiteral("<b>30 s</b>")) || live->body()->text().contains(QStringLiteral("30 s")));
        QVERIFY(chat->stopButton()->isVisible());

        w.window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "done"}, {"text", "Cut the harbor takes to **30 s**."}});
        w.window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", false}});
        settle();
        QVERIFY(!chat->turnLive());
        QVERIFY(!live->work()->isLive());
        QVERIFY(live->work()->headline().startsWith(QStringLiteral("Worked for")));

        // Kept: the conversation's messages, and the live ones gone.
        w.window.handleEvent(conversation("c-9", "Make a 30-second cut",
                                          {item("i1", "you", "Make a 30-second cut from the harbor takes"),
                                           item("i2", "kerf", "Cut the harbor takes to **30 s**.",
                                                {{"steps", QJsonArray{"look at the harbor takes"}}, {"seconds", 41.0}})}));
        settle();
        QVERIFY(!chat->liveMessage());
        QCOMPARE(chat->messages().size(), 2);
        QCOMPARE(chat->answers().size(), 1);
        QCOMPARE(chat->answers().first()->work()->headline(), QStringLiteral("Worked for 41s · 1 step"));
        QCOMPARE(chat->transcript(), QStringLiteral("You: Make a 30-second cut from the harbor takes\nKerf: Cut the harbor takes to **30 s**."));
        // The request kept is the one shown at once: the same widget.
        QCOMPARE(chat->requests().size(), 1);

        // The next request goes on in it.
        chat->composerWidget()->fill(QStringLiteral("Shorter"));
        QVERIFY(w.window.m_chatWiring->post());
        QCOMPARE(w.of("agent").last().value("conversation").toString(), QStringLiteral("c-9"));
        settle();
        QCOMPARE(chat->requests().size(), 2);
        // A message kept again keeps the widgets already shown.
        QWidget *first = chat->messages().first();
        w.window.handleEvent(conversation("c-9", "Make a 30-second cut",
                                          {item("i1", "you", "Make a 30-second cut from the harbor takes"),
                                           item("i2", "kerf", "Cut the harbor takes to **30 s**."), item("i3", "you", "Shorter")}));
        settle();
        QCOMPARE(chat->messages().first(), first);
        QCOMPARE(chat->requests().size(), 2);
        w.window.close();
    }

    // A failed or stopped turn says so and offers Ask again, which asks the
    // same in the same conversation.
    void aFailedTurnOffersAskAgain() {
        Window w;
        w.window.m_chatWiring->openConversation(QStringLiteral("c-1"));
        QCOMPARE(w.of("conversations").last().value("action").toString(), QStringLiteral("open"));
        QCOMPARE(w.of("conversations").last().value("id").toString(), QStringLiteral("c-1"));
        w.window.handleEvent(conversation("c-1", "Limit the music",
                                          {item("i1", "you", "Limit the music bus"),
                                           item("i2", "kerf", "A2 Music is locked.", {{"error", true}, {"seconds", 2.0}}),
                                           item("i3", "you", "Try again"), item("i4", "kerf", "Stopped before finishing.", {{"stopped", true}})}));
        settle();
        const QList<KerfMessage *> answers = w.chat()->answers();
        QCOMPARE(answers.size(), 2);
        QCOMPARE(answers.at(0)->outcome(), KerfMessage::Outcome::Failed);
        QCOMPARE(answers.at(1)->outcome(), KerfMessage::Outcome::Stopped);
        QVERIFY(!answers.at(0)->askAgainButton()->isHidden());
        w.sent.clear();
        answers.at(0)->askAgainButton()->click();
        QCOMPARE(w.of("agent").size(), 1);
        QCOMPARE(w.of("agent").first().value("text").toString(), QStringLiteral("Limit the music bus"));
        QCOMPARE(w.of("agent").first().value("conversation").toString(), QStringLiteral("c-1"));
        // A turn that fails live says so where it is.
        w.window.handleEvent(QJsonObject{{"type", "agent_state"}, {"state", "failed"}, {"message", "The model is offline"}});
        settle();
        QCOMPARE(w.chat()->liveMessage()->outcome(), KerfMessage::Outcome::Failed);
        QCOMPARE(w.chat()->liveMessage()->text(), QStringLiteral("The model is offline"));
    }

    // The sidebar lists the conversations; choosing one opens it, and archive,
    // rename and delete are sent to the bridge.
    void theSidebarKeepsTheConversations() {
        Window w;
        w.window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&w.window));
        // Connected, the editor asks for the list.
        w.window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "ready"}});
        QVERIFY(!w.of("conversations").isEmpty());
        QCOMPARE(w.of("conversations").first().value("action").toString(), QStringLiteral("list"));
        w.window.handleEvent(listed({summary("c-1", "Cut the harbor", 4), summary("c-2", "Captions", 2, true)}));
        settle();
        ProjectSidebar *sidebar = w.window.m_sidebar;
        QCOMPARE(sidebar->conversations().size(), 2);
        QCOMPARE(sidebar->conversations().first().title, QStringLiteral("Cut the harbor"));
        QVERIFY(sidebar->conversations().at(1).archived);
        QCOMPARE(sidebar->conversations().first().count, 4);
        QVERIFY(sidebar->conversations().first().updated.isValid());

        emit sidebar->conversationChosen(QStringLiteral("c-1"));
        QCOMPARE(w.conversations()->currentId(), QStringLiteral("c-1"));
        QCOMPARE(w.of("conversations").last(), (QJsonObject{{"type", "conversations"}, {"action", "open"}, {"id", "c-1"}}));
        QCOMPARE(w.chat()->title()->property("fullTitle").toString(), QStringLiteral("Cut the harbor"));

        emit sidebar->archiveRequested(QStringLiteral("c-2"), false);
        QCOMPARE(w.of("conversations").last(),
                 (QJsonObject{{"type", "conversations"}, {"action", "archive"}, {"id", "c-2"}, {"archived", false}}));
        emit sidebar->renameRequested(QStringLiteral("c-1"), QStringLiteral("Harbor cut"));
        QCOMPARE(w.of("conversations").last(),
                 (QJsonObject{{"type", "conversations"}, {"action", "rename"}, {"id", "c-1"}, {"title", "Harbor cut"}}));
        emit sidebar->deleteRequested(QStringLiteral("c-1"));
        QCOMPARE(w.of("conversations").last(), (QJsonObject{{"type", "conversations"}, {"action", "delete"}, {"id", "c-1"}}));
        // The one open goes at once.
        QVERIFY(w.conversations()->currentId().isEmpty());
        QVERIFY(w.chat()->welcome()->isVisible());

        // New conversation: none open, and the next request starts one.
        emit sidebar->conversationChosen(QStringLiteral("c-2"));
        emit sidebar->newConversationRequested();
        QVERIFY(w.conversations()->currentId().isEmpty());
        w.window.close();
    }

    // The header's menu renames the open conversation in place, archives it,
    // and asks before deleting it.
    void theHeaderMenuActsOnTheOpenConversation() {
        Window w;
        QMenu *none = w.chat()->buildMoreMenu();
        QVERIFY(!none->findChild<QAction *>(QStringLiteral("chatRename"))->isEnabled());
        delete none;
        w.window.m_chatWiring->openConversation(QStringLiteral("c-1"));
        w.window.handleEvent(conversation("c-1", "Cut the harbor", {item("i1", "you", "Cut it")}));
        settle();
        QMenu *menu = w.chat()->buildMoreMenu();
        QAction *rename = menu->findChild<QAction *>(QStringLiteral("chatRename"));
        QAction *archive = menu->findChild<QAction *>(QStringLiteral("chatArchive"));
        QVERIFY(rename->isEnabled() && archive->isEnabled());
        QCOMPARE(archive->text(), QStringLiteral("Archive"));
        archive->trigger();
        QCOMPARE(w.of("conversations").last(),
                 (QJsonObject{{"type", "conversations"}, {"action", "archive"}, {"id", "c-1"}, {"archived", true}}));
        rename->trigger();
        QVERIFY(!w.chat()->titleEditor()->isHidden());
        QCOMPARE(w.chat()->titleEditor()->text(), QStringLiteral("Cut the harbor"));
        w.chat()->titleEditor()->setText(QStringLiteral("Harbor cut"));
        emit w.chat()->titleEditor()->editingFinished();
        QVERIFY(w.chat()->titleEditor()->isHidden());
        QCOMPARE(w.of("conversations").last(),
                 (QJsonObject{{"type", "conversations"}, {"action", "rename"}, {"id", "c-1"}, {"title", "Harbor cut"}}));
        delete menu;
    }

    // The composer says how far Kerf may go without asking, and each request
    // goes with it: chosen from the chip's menu, with Shift-Tab, or in
    // Settings, and kept for the next window.
    void aRequestGoesWithHowFarKerfMayGo() {
        QSettings().remove(QStringLiteral("agent/permissions"));
        const auto forget = qScopeGuard([] { QSettings().remove(QStringLiteral("agent/permissions")); });
        const auto ask = [](Window &w, const char *text) {
            w.sent.clear();
            w.window.m_chatInput->setPlainText(QString::fromLatin1(text));
            const bool sent = w.window.m_chatWiring->post();
            // The turn ends: the next can be asked.
            w.window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", false}});
            return sent ? w.of("agent").value(0).value("permissions").toString() : QStringLiteral("not sent");
        };
        {
            Window w;
            Composer *composer = w.chat()->composerWidget();
            ComposerChip *chip = composer->modeChip();
            // Until chosen otherwise, every edit is a proposal.
            QVERIFY(!chip->isHidden());
            QCOMPARE(w.chat()->permissionMode(), PermissionMode::Ask);
            QCOMPARE(chip->text(), QStringLiteral("Ask before edits"));
            QVERIFY(!chip->tone().isValid());
            QVERIFY(w.chat()->findChild<QLabel *>(QStringLiteral("welcomeText"))->text().contains(QStringLiteral("proposal")));
            QCOMPARE(ask(w, "Trim the intro"), QStringLiteral("ask"));

            // From the chip's menu: the three, the one in force checked, each with what it lets through.
            QMenu *menu = composer->buildModeMenu();
            QAction *asks = menu->findChild<QAction *>(QStringLiteral("composerMode.ask"));
            QAction *edits = menu->findChild<QAction *>(QStringLiteral("composerMode.accept-edits"));
            QAction *automatic = menu->findChild<QAction *>(QStringLiteral("composerMode.auto"));
            QVERIFY(asks && edits && automatic);
            QVERIFY(asks->isChecked() && !edits->isChecked() && !automatic->isChecked());
            QCOMPARE(ui::menuKey(edits), QStringLiteral("Edits apply as Kerf makes them"));
            edits->trigger();
            delete menu;
            QCOMPARE(w.chat()->permissionMode(), PermissionMode::AcceptEdits);
            QCOMPARE(chip->text(), QStringLiteral("Accept edits"));
            // The more Kerf does on its own, the more the chip says so.
            QCOMPARE(chip->tone(), tokens::color(tokens::accentInk));
            QCOMPARE(QSettings().value(QStringLiteral("agent/permissions")).toString(), QStringLiteral("accept-edits"));
            // The welcome no longer promises a proposal first, and Settings shows the same choice.
            QVERIFY(w.chat()->findChild<QLabel *>(QStringLiteral("welcomeText"))->text().contains(QStringLiteral("Undo")));
            QCOMPARE(w.window.m_permissionChoice->currentIndex(), 1);
            QCOMPARE(ask(w, "Trim the intro"), QStringLiteral("accept-edits"));

            // Shift-Tab in the text goes round the three; Tab still leaves it.
            QTest::keyClick(w.window.m_chatInput, Qt::Key_Backtab, Qt::ShiftModifier);
            QCOMPARE(w.chat()->permissionMode(), PermissionMode::Auto);
            QCOMPARE(chip->text(), QStringLiteral("Auto"));
            QCOMPARE(chip->tone(), tokens::color(tokens::activity::gpu));
            QCOMPARE(QSettings().value(QStringLiteral("agent/permissions")).toString(), QStringLiteral("auto"));
            QCOMPARE(w.window.m_permissionChoice->currentIndex(), 2);
            QCOMPARE(ask(w, "Make the opener"), QStringLiteral("auto"));
            QTest::keyClick(w.window.m_chatInput, Qt::Key_Backtab, Qt::ShiftModifier);
            QCOMPARE(w.chat()->permissionMode(), PermissionMode::Ask);

            // And from Settings, where the choice is spelled out.
            w.window.m_permissionChoice->setCurrentIndex(1, false);
            emit w.window.m_permissionChoice->activated(1);
            QCOMPARE(w.chat()->permissionMode(), PermissionMode::AcceptEdits);
            QCOMPARE(chip->text(), QStringLiteral("Accept edits"));
            QVERIFY(w.window.m_permissionHint->text().contains(QStringLiteral("Undo")));
            QCOMPARE(QSettings().value(QStringLiteral("agent/permissions")).toString(), QStringLiteral("accept-edits"));
        }
        // The next window starts as this one was left.
        {
            Window next;
            QCOMPARE(next.chat()->permissionMode(), PermissionMode::AcceptEdits);
            QCOMPARE(next.window.m_permissionChoice->currentIndex(), 1);
            QCOMPARE(ask(next, "Trim it"), QStringLiteral("accept-edits"));
        }
        // A name the settings hold that is no mode asks.
        QCOMPARE(permission::named(QStringLiteral("everything")), PermissionMode::Ask);
        for (const PermissionMode mode : {PermissionMode::Ask, PermissionMode::AcceptEdits, PermissionMode::Auto})
            QCOMPARE(permission::named(permission::name(mode)), mode);
    }

    // The composer names the model that answers, and offers the others where
    // there is a choice; Kerf Cloud picks its own, so its chip only says.
    void theComposerNamesTheModel() {
        Window w;
        Composer *composer = w.chat()->composerWidget();
        ComposerChip *chip = composer->modelChip();
        w.chat()->setModels({QStringLiteral("Kerf Cloud")}, QStringLiteral("Kerf Cloud"), false);
        QVERIFY(!chip->isHidden());
        QCOMPARE(chip->text(), QStringLiteral("Kerf Cloud"));
        QVERIFY(!chip->hasChevron());
        w.chat()->setModels({QStringLiteral("claude-opus-5-5"), QStringLiteral("claude-sonnet-5-5")}, QStringLiteral("claude-opus-5-5"), true);
        QCOMPARE(chip->text(), QStringLiteral("claude-opus-5-5"));
        QVERIFY(chip->hasChevron());
        QMenu *menu = composer->buildModelMenu();
        QStringList offered;
        QAction *sonnet = nullptr;
        for (QAction *action : menu->actions()) {
            if (!action->isCheckable()) continue;
            offered << action->text();
            if (action->text() == QStringLiteral("claude-sonnet-5-5")) sonnet = action;
            QCOMPARE(action->isChecked(), action->text() == QStringLiteral("claude-opus-5-5"));
        }
        QCOMPARE(offered, (QStringList{QStringLiteral("claude-opus-5-5"), QStringLiteral("claude-sonnet-5-5")}));
        QVERIFY(menu->findChild<QAction *>(QStringLiteral("composerModelSettings")));
        // Chosen, it is the one the next request names.
        QVERIFY(sonnet);
        sonnet->trigger();
        delete menu;
        QCOMPARE(w.window.m_agentModel, QStringLiteral("claude-sonnet-5-5"));
        QCOMPARE(chip->text(), QStringLiteral("claude-sonnet-5-5"));
        // With no model to name, it says nothing.
        w.chat()->setModels({}, QString(), false);
        QVERIFY(chip->isHidden());
    }

    // How full Kerf's context is shows beside Send, for the conversation the
    // turn was in: a ring that turns amber, then red, as it fills.
    void theMeterSaysHowFullTheContextIs() {
        Window w;
        ContextMeter *meter = w.chat()->composerWidget()->contextMeter();
        QVERIFY(meter->isHidden());
        w.window.m_chatWiring->openConversation(QStringLiteral("c-1"));
        w.window.handleEvent(conversation("c-1", "Cut the harbor", {item("i1", "you", "Cut it"), item("i2", "kerf", "Done.")}));
        settle();
        QVERIFY(meter->isHidden());
        const auto reading = [&w](qint64 used, qint64 limit, const char *unit, const char *conversation) {
            w.window.handleEvent(QJsonObject{{"type", "agent_context"}, {"used", used}, {"limit", limit}, {"unit", unit}, {"conversation", conversation}});
        };
        reading(96'000, 192'000, "units", "c-1");
        QVERIFY(!meter->isHidden());
        QCOMPARE(meter->fraction(), 0.5);
        QCOMPARE(meter->ringColor(), tokens::color(tokens::ink2));
        // Kerf's own measure is a share, with what happens as it fills.
        QVERIFY2(meter->describe().startsWith(QStringLiteral("Context 50% full")), qPrintable(meter->describe()));
        QVERIFY(!meter->describe().contains(QStringLiteral("tokens")));
        QCOMPARE(meter->toolTip(), meter->describe());
        reading(150'000, 192'000, "units", "c-1");
        QCOMPARE(meter->ringColor(), tokens::color(tokens::warn));
        reading(180'000, 192'000, "units", "c-1");
        QCOMPARE(meter->ringColor(), tokens::color(tokens::neg));
        // A model's window is counted in its tokens.
        reading(68'000, 200'000, "tokens", "c-1");
        QCOMPARE(meter->describe(), QStringLiteral("Context 34% full · 68k of 200k tokens"));
        reading(1'200'000, 1'000'000, "tokens", "c-1");
        QCOMPARE(meter->fraction(), 1.0);

        // It is that conversation's: another has none to show, and coming back has it again.
        w.window.m_chatWiring->openConversation(QStringLiteral("c-2"));
        w.window.handleEvent(conversation("c-2", "Captions", {item("i1", "you", "Caption it")}));
        settle();
        QVERIFY(meter->isHidden());
        w.window.m_chatWiring->openConversation(QStringLiteral("c-1"));
        settle();
        QVERIFY(!meter->isHidden());
        // A reading for another conversation's turn does not show here.
        reading(10'000, 192'000, "units", "c-9");
        QVERIFY(meter->isHidden());
        // With nothing to be full of, nothing shows.
        reading(0, 0, "units", "c-1");
        QVERIFY(meter->isHidden());
    }

    // What is said is set in the faces chosen in Settings: the interface's, or
    // a serif of its own, at the size chosen; the faces come with the editor.
    void theConversationIsSetInTheChosenFaces() {
        const auto restore = qScopeGuard([] {
            setEditorFaces(FaceChoice{});
            QSettings().remove(QStringLiteral("appearance"));
        });
        // The faces the editor carries are there to choose.
        for (const char *family : {tokens::uiFamily, tokens::hangulFamily, tokens::serifFamily, tokens::serifHangulFamily})
            QVERIFY2(QFontDatabase::families().contains(QString::fromLatin1(family)), family);
        // The design's own, with Pretendard for the Hangul Inter has none of.
        const tokens::Faces plain = facesFor(FaceChoice{});
        QCOMPARE(plain.ui, (QStringList{QStringLiteral("Inter"), QStringLiteral("Pretendard")}));
        QVERIFY(plain.prose.isEmpty());
        QCOMPARE(plain.proseStep, 0);
        const FaceChoice chosen{QStringLiteral("pretendard"), QStringLiteral("serif"), QStringLiteral("large")};
        const tokens::Faces fancy = facesFor(chosen);
        QCOMPARE(fancy.ui, (QStringList{QStringLiteral("Pretendard"), QStringLiteral("Inter")}));
        QCOMPARE(fancy.prose, (QStringList{QStringLiteral("Noto Serif"), QStringLiteral("NanumMyeongjo")}));
        QCOMPARE(fancy.proseStep, 2);
        QCOMPARE(facesFor(FaceChoice{QStringLiteral("system"), QStringLiteral("same"), QStringLiteral("small")}).proseStep, -1);
        // Kept and read back; a name that is none of them is the first of its kind.
        saveFaceChoice(chosen);
        QCOMPARE(loadFaceChoice(), chosen);
        QSettings().setValue(QStringLiteral("appearance/interfaceFace"), QStringLiteral("comic"));
        QSettings().setValue(QStringLiteral("appearance/readingSize"), QStringLiteral("huge"));
        QCOMPARE(loadFaceChoice(), (FaceChoice{QStringLiteral("inter"), QStringLiteral("serif"), QStringLiteral("default")}));
        QSettings().remove(QStringLiteral("appearance"));

        Window w;
        w.window.m_chatWiring->openConversation(QStringLiteral("c-1"));
        w.window.handleEvent(conversation("c-1", "Cut the harbor", {item("i1", "you", "Cut it"), item("i2", "kerf", "Done, in **one** pass.")}));
        settle();
        const auto first = [](const QFont &font) { return font.families().value(0); };
        QCOMPARE(first(w.chat()->answers().constFirst()->body()->font()), QStringLiteral("Inter"));
        QCOMPARE(w.chat()->answers().constFirst()->body()->font().pixelSize(), 14);
        QCOMPARE(first(w.chat()->title()->font()), QStringLiteral("Inter"));

        // Chosen in Settings: put in force at once, and kept.
        w.window.m_interfaceFace->setCurrentIndex(1, false);
        w.window.m_readingFace->setCurrentIndex(1, false);
        w.window.m_readingSize->setCurrentIndex(2, false);
        emit w.window.m_readingSize->activated(2);
        settle();
        QCOMPARE(loadFaceChoice(), chosen);
        // What is asked and answered, and what is being written, are in the serif, larger.
        QCOMPARE(w.chat()->answers().size(), 1);
        QCOMPARE(first(w.chat()->answers().constFirst()->body()->font()), QStringLiteral("Noto Serif"));
        QCOMPARE(w.chat()->answers().constFirst()->body()->font().pixelSize(), 16);
        QCOMPARE(first(w.chat()->requests().constFirst()->body()->font()), QStringLiteral("Noto Serif"));
        QCOMPARE(first(w.window.m_chatInput->font()), QStringLiteral("Noto Serif"));
        QCOMPARE(w.window.m_chatInput->font().pixelSize(), 16);
        QCOMPARE(first(w.window.m_readingSample->font()), QStringLiteral("Noto Serif"));
        // The rest of the editor is in the interface's face, at its own sizes.
        QCOMPARE(first(w.chat()->title()->font()), QStringLiteral("Pretendard"));
        QCOMPARE(w.chat()->title()->font().pixelSize(), tokens::title.px);
        QCOMPARE(first(tokens::font(tokens::control)), QStringLiteral("Pretendard"));
        QCOMPARE(first(QApplication::font()), QStringLiteral("Pretendard"));
        QCOMPARE(first(tokens::font(tokens::timecode)), QStringLiteral("JetBrains Mono"));

        // Back to the design's own.
        w.window.m_interfaceFace->setCurrentIndex(0, false);
        w.window.m_readingFace->setCurrentIndex(0, false);
        w.window.m_readingSize->setCurrentIndex(1, false);
        emit w.window.m_interfaceFace->activated(0);
        settle();
        QCOMPARE(first(w.chat()->answers().constFirst()->body()->font()), QStringLiteral("Inter"));
        QCOMPARE(w.chat()->answers().constFirst()->body()->font().pixelSize(), 14);
        QCOMPARE(first(w.chat()->title()->font()), QStringLiteral("Inter"));
        QCOMPARE(first(w.window.m_chatInput->font()), QStringLiteral("Inter"));
    }

    // A proposal a turn staged has its card under the answer; one staged
    // elsewhere (an agent over MCP) waits at the end, saying who made it.
    void cardsSitUnderTheAnswerThatStagedThem() {
        Window w;
        const QJsonObject program{{"fps", 25}, {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
            QJsonObject{{"id", "sea"}, {"name", "B003_C011"}, {"timelineStart", 0.0}, {"duration", 4.0}}}}}}}};
        QJsonObject trimmed = program;
        {
            QJsonArray tracks = trimmed["tracks"].toArray();
            QJsonObject v1 = tracks[0].toObject();
            QJsonArray clips = v1["clips"].toArray();
            QJsonObject sea = clips[0].toObject();
            sea["duration"] = 3.0;
            clips[0] = sea;
            v1["clips"] = clips;
            tracks[0] = v1;
            trimmed["tracks"] = tracks;
        }
        const auto entry = [](const char *id, const QJsonObject &author) {
            return QJsonObject{{"proposalId", id}, {"status", "pending"}, {"freshness", "current"}, {"baseRevision", 3},
                               {"script", "trim sea out -25f\n"}, {"author", author}, {"version", 1}};
        };
        const QJsonObject snapshot{{"revision", 3}, {"duration", 4}, {"timeline", program},
                                   {"proposals", QJsonArray{entry("p-kerf", QJsonObject{{"kind", "kerf"}, {"name", "Kerf"}}),
                                                            entry("p-cc", QJsonObject{{"kind", "mcp"}, {"name", "Claude Code"}})}},
                                   {"proposalTimelines", QJsonObject{{"p-kerf", trimmed}, {"p-cc", trimmed}}}};
        w.window.applySnapshot(snapshot);
        w.window.m_chatWiring->handleEvent(QStringLiteral("state"), QJsonObject{{"type", "state"}, {"snapshot", snapshot}});
        w.window.m_chatWiring->openConversation(QStringLiteral("c-1"));
        w.window.handleEvent(conversation("c-1", "Trim the sea",
                                          {item("i1", "you", "Trim the sea"), item("i2", "kerf", "Trimmed a second.", {{"proposal", "p-kerf"}})}));
        settle();
        KerfMessage *answer = w.chat()->answers().first();
        QCOMPARE(answer->cards().size(), 1);
        ProposalCard *card = w.chat()->card(QStringLiteral("p-kerf"));
        QVERIFY(card && answer->isAncestorOf(card));
        ProposalCard *stray = w.chat()->card(QStringLiteral("p-cc"));
        QVERIFY(stray && !answer->isAncestorOf(stray));
        QCOMPARE(w.chat()->answers().constLast()->text(), QStringLiteral("Claude Code proposed this over MCP."));
        // Accept from its card is sent, and the card settles.
        w.sent.clear();
        emit card->acceptRequested(QStringLiteral("p-kerf"));
        QCOMPARE(w.of("accept").size(), 1);
        QCOMPARE(w.of("accept").first().value("proposalId").toString(), QStringLiteral("p-kerf"));
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::ChatTest)

#include "chat_test.moc"
