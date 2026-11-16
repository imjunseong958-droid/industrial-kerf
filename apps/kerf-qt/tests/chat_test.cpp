// Tests for the conversation: the conversations kept beside the project
// (ui/conversation_model.h) as the bridge sends them, the panel that reads
// one top to bottom (chat_panel.h, ui/chat_messages.h), and the wiring that
// sends what is asked (agent, conversations) and feeds the sidebar.

#include "fixtures.h"
#include "test_registry.h"

#include "chat_panel.h"
#include "components.h"
#include "composer.h"
#include "main_window.h"
#include "project_sidebar.h"
#include "ui/chat_messages.h"
#include "ui/chat_parts.h"
#include "ui/conversation_model.h"
#include "ui/editor_bus.h"
#include "ui/wiring/chat_wiring.h"

#include <QAbstractButton>
#include <QCoreApplication>
#include <QJsonArray>
#include <QJsonObject>
#include <QLabel>
#include <QMenu>
#include <QSignalSpy>
#include <QTest>
#include <QTextEdit>

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
        QCOMPARE(model.list().first().updated, QDateTime(QDate(2026, 11, 16), QTime(9, 5), QTimeZone::UTC));
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
