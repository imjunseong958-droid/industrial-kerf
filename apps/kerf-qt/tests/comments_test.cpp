// Tests for the Comments panel (the agent chat, mockup-spec 4): its threads,
// chips, filters, pin bar, proposal cards with versions and the colour chip,
// the composer with its stamp, mention and draw, the Ask popover, and how
// MainWindow wires them to kerfd (the agent turn kept as comments,
// comment_add, comment_set, accept/reject, preview, effect_previews).
//
// The fixtures are the mockup's seed threads (spec 4.9) as the bridge sends
// them; nothing here is sample data in the app.

#include "fixtures.h"
#include "test_registry.h"

#include "ask_prompt.h"
#include "chat_panel.h"
#include "stage_shell.h"
#include "command_registry.h"
#include "comment_row.h"
#include "components.h"
#include "composer.h"
#include "editor_bus.h"
#include "main_window.h"
#include "proposal_card.h"
#include "review_model.h"
#include "viewer_widget.h"
#include "wiring/review_wiring.h"

#include <QAbstractButton>
#include <QAction>
#include <QApplication>
#include <QClipboard>
#include <QComboBox>
#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QGroupBox>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMenu>
#include <QPushButton>
#include <QRadioButton>
#include <QScrollArea>
#include <QScrollBar>
#include <QSettings>
#include <QEnterEvent>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QTest>
#include <QTextEdit>
#include <QScopeGuard>
#include <QTextDocument>

#include <algorithm>
#include <cmath>
#include <initializer_list>
#include <limits>

namespace kerf {

using namespace test;

namespace {

const QString kYou = QStringLiteral("ana@kerf.test");
const QString kMina = QStringLiteral("mina@kerf.test");
const QString kTom = QStringLiteral("tom@kerf.test");

QJsonObject person(const char *name, const QString &email, const char *role = nullptr) {
    QJsonObject author{{"kind", "person"}, {"name", name}, {"id", email}};
    if (role) author.insert("role", role);
    return author;
}

QJsonObject agent(const char *kind, const char *name, const char *id, const char *role) {
    QJsonObject author{{"kind", kind}, {"name", name}, {"role", role}};
    if (id) author.insert("id", id);
    return author;
}

QJsonObject mina() { return person("Mina Park", kMina, "director"); }
QJsonObject tom() { return person("Tom Lind", kTom, "colourist"); }
QJsonObject you() { return person("Ana Ruiz", kYou); }
QJsonObject kerfAuthor(const char *role = nullptr) {
    QJsonObject author{{"kind", "kerf"}, {"name", "Kerf"}};
    if (role) author.insert("role", role);
    return author;
}

QJsonObject note(const char *id, const QJsonObject &author, const char *created, const char *body,
                 std::initializer_list<std::pair<const char *, QJsonValue>> extra = {}) {
    QJsonObject record{{"id", id},           {"parent", QJsonValue()}, {"author", author}, {"body", body},
                       {"tags", QJsonArray{}}, {"blocking", false},    {"resolved", false}, {"created", created},
                       {"revision", 22}};
    for (const auto &[key, value] : extra) record.insert(key, value);
    return record;
}

QJsonObject reply(const char *id, const char *parent, const QJsonObject &author, const char *created, const char *body,
                  std::initializer_list<std::pair<const char *, QJsonValue>> extra = {}) {
    QJsonObject record = note(id, author, created, body, extra);
    record.insert("parent", parent);
    return record;
}

// The seed threads of mockup-spec 4.9 as kerfd's review log folds them (the
// same records review_model_test reads).
QJsonArray seedComments() {
    return QJsonArray{
        note("c-2916", agent("mcp", "Codex", "codex", "MCP"), "2026-11-01T12:00:00.000000Z",
             "End card holds a beat long, trimmed 12 frames",
             {{"at", 29.64}, {"frame", 741}, {"proposal", "proposal-cx"}, {"resolved", true}, {"resolvedRevision", 20}}),
        note("c-0300", mina(), "2026-11-02T09:00:00.000000Z", "Open on the road, not the sky", {{"at", 3.0}, {"frame", 75}}),
        reply("c-0300-r1", "c-0300", agent("mcp", "Cursor", "cursor", "MCP"), "2026-11-02T09:10:00.000000Z",
              "Moved A012_C004 to the head, trimmed 1 s of sky",
              {{"proposal", "proposal-cu"}, {"resolved", true}, {"resolvedBy", you()}, {"resolvedRevision", 20}}),
        reply("c-0300-r2", "c-0300", mina(), "2026-11-02T09:20:00.000000Z", "Better, thanks"),
        note("c-0903", tom(), "2026-11-02T10:00:00.000000Z", "The sea reads magenta here",
             {{"at", 9.12}, {"frame", 228}, {"tags", QJsonArray{"color"}}}),
        reply("c-0903-r1", "c-0903", kerfAuthor(), "2026-11-02T10:01:00.000000Z", "Sea back toward cyan",
              {{"proposal", "proposal-kg"},
               {"steps", QJsonArray{"Sampled the sea on B003_C011 at 00:00:09:03", "Measured hue 9° off the cyan axis",
                                    "Drafted a hue-vs-hue key, sea only"}}}),
        note("c-1708", kerfAuthor("QC"), "2026-11-03T08:00:00.000000Z", "Caption overlaps the lower-third by 8 px",
             {{"at", 17.32}, {"duration", 2.48}, {"frame", 433}, {"tags", QJsonArray{"captions"}},
              {"qcKey", "qc:text.overlap:clip-12"}}),
        reply("c-1708-r1", "c-1708", tom(), "2026-11-03T08:30:00.000000Z", "Also clips the sun flare, circled",
              {{"drawing", QJsonArray{QJsonObject{{"tool", "ellipse"},
                                                  {"points", QJsonArray{QJsonArray{0.61, 0.22}, QJsonArray{0.74, 0.35}}},
                                                  {"color", "#ededed"}}}}}),
        note("c-2614", mina(), "2026-11-03T09:00:00.000000Z", "Logo lands late, should hit on the snare",
             {{"at", 26.56}, {"frame", 664}, {"blocking", true}}),
        reply("c-2614-r1", "c-2614", agent("mcp", "Claude Code", "claude-code", "MCP · via Kerf"),
              "2026-11-03T09:05:00.000000Z", "Move logo reveal to the snare", {{"proposal", "proposal-cc1"}}),
        reply("c-2614-r2", "c-2614", mina(), "2026-11-03T09:10:00.000000Z", "Two frames earlier, on the transient 00:00:26:12"),
        reply("c-2614-r3", "c-2614", agent("mcp", "Claude Code", "claude-code", "MCP"), "2026-11-03T09:15:00.000000Z",
              "Updated to v2, Logo_reveal now lands on 00:00:26:12", {{"proposal", "proposal-cc2"}}),
        note("c-2718", agent("qc", "QC", nullptr, "loudness"), "2026-11-03T10:00:00.000000Z",
             "True peak −0.4 dBTP, limit −1.0 dBTP (EBU R128)",
             {{"at", 27.72}, {"duration", 1.08}, {"frame", 693}, {"blocking", true}, {"tags", QJsonArray{"audio"}},
              {"qcKey", "qc:audio.true_peak"}}),
        reply("c-2718-r1", "c-2718", you(), "2026-11-03T10:05:00.000000Z", "@Kerf limit the music bus here"),
        reply("c-2718-r2", "c-2718", kerfAuthor(), "2026-11-03T10:06:00.000000Z", "A2 Music is locked. Unlock it, then ask again",
              {{"error", true}}),
        reply("c-2718-r3", "c-2718", you(), "2026-11-03T10:10:00.000000Z", "Unlocked A2, try again"),
    };
}

// The program at 25 fps, 30 s: the sea shot, the logo and its whoosh.
QJsonObject timeline(double logoAt, const QJsonArray &seaFx = {}) {
    const auto clip = [](const char *id, const char *name, double start, double duration) {
        return QJsonObject{{"id", id}, {"name", name}, {"kind", "media"}, {"timelineStart", start}, {"duration", duration}};
    };
    QJsonObject sea = clip("sea", "B003_C011", 8.48, 4.72);
    if (!seaFx.isEmpty()) sea.insert("effects", seaFx);
    return QJsonObject{
        {"fps", 25},
        {"tracks", QJsonArray{
                       QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{clip("road", "A012_C004", 0.0, 8.48), sea}}},
                       QJsonObject{{"id", "V4"}, {"type", "video"}, {"clips", QJsonArray{clip("logo", "Logo_reveal", logoAt, 30.0 - logoAt)}}},
                       QJsonObject{{"id", "A3"}, {"type", "audio"}, {"clips", QJsonArray{clip("whoosh", "Whoosh_logo", logoAt + 0.28, 1.0)}}},
                   }},
    };
}

QJsonArray hueFx() {
    return QJsonArray{QJsonObject{{"fx", "color.huehue"}, {"version", 1}, {"enabled", true}, {"params", QJsonObject{{"hue", -9}, {"sat", -6}}}}};
}

QJsonObject proposalEntry(const char *id, const char *status, const QJsonObject &author, int version, const QJsonArray &versions,
                          const char *script, std::initializer_list<std::pair<const char *, QJsonValue>> extra = {}) {
    QJsonObject entry{{"proposalId", id},  {"status", status},   {"freshness", "current"}, {"baseRevision", 22},
                      {"script", script},  {"source", script},   {"documentDiff", ""},     {"author", author},
                      {"version", version}, {"supersedes", QJsonValue()}, {"versions", versions}};
    for (const auto &[key, value] : extra) entry.insert(key, value);
    return entry;
}

QJsonObject versionEntry(int version, const char *id, const char *script, const char *diff = "") {
    return QJsonObject{{"version", version}, {"proposalId", id}, {"baseRevision", 22}, {"script", script}, {"documentDiff", diff}};
}

const char *kLogoDiff = "--- before\n+++ after\n@@ -4,2 +4,2 @@\n- clip \"Logo_reveal\" V4 00:00:26:03\n+ clip \"Logo_reveal\" V4 00:00:25:20\n";

// The snapshot at rev 22: Claude Code's v2 and Kerf's colour key wait; two
// settled at rev 20.
QJsonObject seedSnapshot() {
    const QJsonObject claude = agent("mcp", "Claude Code", "claude-code", "MCP · via Kerf");
    const char *cc1 = "move logo to 00:00:25:21\nmove whoosh to 00:00:26:03\n";
    const char *cc2 = "move logo to 00:00:25:20\nmove whoosh to 00:00:26:02\n";
    const char *kg = "use main\neffect add sea color.huehue hue=-9 sat=-6\n";
    QJsonObject cc = proposalEntry("proposal-cc2", "pending", claude, 2,
                                   QJsonArray{versionEntry(1, "proposal-cc1", cc1), versionEntry(2, "proposal-cc2", cc2, kLogoDiff)},
                                   cc2, {{"supersedes", "proposal-cc1"}, {"staged", "2026-11-03T09:15:00Z"}});
    cc.insert("documentDiff", kLogoDiff);
    const QJsonArray proposals{
        cc,
        proposalEntry("proposal-kg", "pending", QJsonObject{{"kind", "kerf"}, {"name", "Kerf"}}, 1,
                      QJsonArray{versionEntry(1, "proposal-kg", kg)}, kg, {{"staged", "2026-11-02T10:01:00Z"}}),
        proposalEntry("proposal-cu", "accepted", agent("mcp", "Cursor", "cursor", "MCP"), 1,
                      QJsonArray{versionEntry(1, "proposal-cu", "move road to 00:00:00:00\n")}, "move road to 00:00:00:00\n",
                      {{"appliedRevision", 20}}),
        proposalEntry("proposal-cx", "accepted", agent("mcp", "Codex", "codex", "MCP"), 1,
                      QJsonArray{versionEntry(1, "proposal-cx", "trim end out -12f\n")}, "trim end out -12f\n",
                      {{"appliedRevision", 20}}),
    };
    const QJsonObject timelines{
        {"proposal-cc2", timeline(25.8)},
        {"proposal-cc1", timeline(25.84)},
        {"proposal-kg", timeline(26.12, hueFx())},
    };
    return QJsonObject{{"revision", 22},
                       {"timeline", timeline(26.12)},
                       {"proposals", proposals},
                       {"proposalTimelines", timelines},
                       {"reviewRevision", 41}};
}

QJsonObject identity() { return QJsonObject{{"name", "Ana Ruiz"}, {"email", kYou}}; }

// A panel on its own with the seed, as the editor shows it at the mockup's start.
struct Seeded {
    ReviewModel model;
    ChatPanel panel;
    Seeded(int width = 368, int height = 548) {
        panel.setReview(&model);
        model.setIdentity(identity());
        model.setSnapshot(seedSnapshot());
        model.setComments(seedComments());
        model.setPlayhead(17.32);
        panel.setFps(25);
        panel.setSnapshot(seedSnapshot());
        panel.setPlayhead(17.32);
        panel.setConnected(true);
        panel.resize(width, height);
        panel.show();
        settle();
    }
    void settle() {
        for (int i = 0; i < 4; ++i) QCoreApplication::processEvents();
    }
};

QStringList threadIds(ChatPanel &panel) {
    QStringList ids;
    for (ThreadWidget *thread : panel.threadWidgets()) ids << thread->threadId();
    return ids;
}

// The threads shown, without the welcome (which no filter but From Kerf keeps).
QStringList recordIds(ChatPanel &panel) {
    QStringList ids = threadIds(panel);
    ids.removeAll(QString::fromLatin1(ReviewModel::kWelcomeId));
    return ids;
}

QStringList tagTexts(CommentRow *row) {
    QStringList texts;
    for (TagButton *tag : row->tags()) texts << tag->text();
    return texts;
}

QMenu *openMenu(const QString &name) {
    for (QWidget *widget : QApplication::topLevelWidgets())
        if (auto *menu = qobject_cast<QMenu *>(widget); menu && menu->objectName() == name && menu->isVisible()) return menu;
    return nullptr;
}

QAction *actionNamed(QMenu *menu, const QString &prefix) {
    for (QAction *action : menu->actions())
        if (action->text().startsWith(prefix)) return action;
    return nullptr;
}

QImage solid(QColor color) {
    QImage image(32, 18, QImage::Format_RGBA8888);
    image.fill(color);
    return image;
}

} // namespace

class CommentsTest final : public QObject {
    Q_OBJECT

    // A window fed the seed through the bus as kerfd sends it, its commands
    // caught instead of sent.
    struct SeededWindow {
        MainWindow window{QString(), QString()};
        QList<QJsonObject> sent;
        explicit SeededWindow(bool show = true) {
            window.m_bus->setSender([this](const QJsonObject &command) {
                sent << command;
                window.m_lastCommand = command;
                return true;
            });
            window.setConnection(QStringLiteral("ready"));
            window.m_reviewWiring->handleEvent(QStringLiteral("connection"), QJsonObject{{"type", "connection"}, {"status", "ready"}});
            ReviewWiring *wiring = window.m_reviewWiring.get();
            wiring->handleEvent(QStringLiteral("collab"), QJsonObject{{"type", "collab"}, {"status", QJsonObject{{"user", identity()}}}});
            wiring->handleEvent(QStringLiteral("state"), QJsonObject{{"type", "state"}, {"snapshot", seedSnapshot()}});
            wiring->handleEvent(QStringLiteral("comments"), QJsonObject{{"type", "comments"}, {"items", seedComments()}, {"reviewRevision", 41}});
            wiring->model()->setPlayhead(17.32);
            window.m_chat->setFps(25);
            window.m_fps = 25;
            window.m_revision = 22;
            if (show) {
                window.resize(1600, 1000);
                window.show();
            }
            settle();
        }
        void settle() {
            for (int i = 0; i < 4; ++i) QCoreApplication::processEvents();
        }
        QJsonObject last(const QString &type) const {
            for (auto it = sent.crbegin(); it != sent.crend(); ++it)
                if (it->value("type").toString() == type) return *it;
            return {};
        }
    };

private slots:
    // Each test starts with the panel's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    // ---- agent settings (unchanged protocol) ----------------------------------

    void inboundMcpConfigValidatesStdioShape() {
        QVERIFY(!validateInboundMcpConfig("[]").isEmpty());
        QVERIFY(!validateInboundMcpConfig("{}").isEmpty());
        QVERIFY(!validateInboundMcpConfig("{broken").isEmpty());
        const QByteArray remote = R"({"mcpServers":{"remote":{"url":"https://example.com/mcp"}}})";
        const QByteArray emptyCommand = R"({"mcpServers":{"local":{"command":" "}}})";
        const QByteArray invalidArgs = R"({"mcpServers":{"local":{"command":"server","args":[1]}}})";
        const QByteArray invalidEnv = R"({"mcpServers":{"local":{"command":"server","env":{"TOKEN":1}}}})";
        const QByteArray valid = R"({"mcpServers":{"local":{"command":"server","args":["--stdio"],"env":{"TOKEN":"value"},"tools":["search"]}}})";
        QVERIFY(validateInboundMcpConfig(remote).contains(QStringLiteral("URL-only")));
        QVERIFY(!validateInboundMcpConfig(emptyCommand).isEmpty());
        QVERIFY(!validateInboundMcpConfig(invalidArgs).isEmpty());
        QVERIFY(!validateInboundMcpConfig(invalidEnv).isEmpty());
        QVERIFY(validateInboundMcpConfig(valid).isEmpty());
    }

    void inboundMcpImportRequiresSaveAndInvalidSavePreservesConfig() {
        QTemporaryDir project;
        QVERIFY(project.isValid());
        QVERIFY(QDir(project.path()).mkpath(QStringLiteral(".cursor")));
        const QByteArray config = R"({"mcpServers":{"local":{"command":"server","args":["--stdio"]}}})";
        QFile source(QDir(project.path()).filePath(QStringLiteral(".cursor/mcp.json")));
        QVERIFY(source.open(QIODevice::WriteOnly));
        QCOMPARE(source.write(config), qint64(config.size()));
        source.close();
        const QString savedPath = QDir(project.path()).filePath(QStringLiteral(".kerf/mcp.json"));
        MainWindow window{project.path(), QString()};
        const bool originalEnabled = window.m_mcpEnabled->isChecked();
        window.m_mcpEnabled->setChecked(!originalEnabled);
        QCOMPARE(QSettings().value(QStringLiteral("agent/mcpEnabled")).toBool(), !originalEnabled);
        QCOMPARE(window.agentSettingsPayload().value(QStringLiteral("mcpEnabled")).toBool(), !originalEnabled);
        window.m_mcpEnabled->setChecked(originalEnabled);
        QCOMPARE(window.agentSettingsPayload().value(QStringLiteral("mcpConfigVersion")).toInteger(), qint64(0));
        auto *load = window.findChild<QPushButton *>(QStringLiteral("loadProjectCursorMcp"));
        QVERIFY(load);
        QVERIFY(window.findChild<QPushButton *>(QStringLiteral("loadHomeCursorMcp")));
        load->click();
        QCOMPARE(window.m_inboundMcpJson->toPlainText().toUtf8(), config);
        QVERIFY(!QFileInfo::exists(savedPath));
        auto *save = window.findChild<QPushButton *>(QStringLiteral("saveInboundMcpConfig"));
        QVERIFY(save);
        save->click();
        QCOMPARE(window.agentSettingsPayload().value(QStringLiteral("mcpConfigVersion")).toInteger(), qint64(1));
        QFile saved(savedPath);
        QVERIFY(saved.open(QIODevice::ReadOnly));
        QCOMPARE(saved.readAll(), config);
        saved.close();
        window.m_inboundMcpJson->setPlainText(QStringLiteral("[]"));
        save->click();
        QCOMPARE(window.agentSettingsPayload().value(QStringLiteral("mcpConfigVersion")).toInteger(), qint64(1));
        QVERIFY(saved.open(QIODevice::ReadOnly));
        QCOMPARE(saved.readAll(), config);
        MainWindow reopened{project.path(), QString()};
        QCOMPARE(reopened.m_inboundMcpJson->toPlainText().toUtf8(), config);
    }

    void mcpConfigCopiesProjectWithoutCredentials() {
        const QString projectPath = QDir::temp().filePath(QStringLiteral("Kerf project \"quoted\""));
        MainWindow window{projectPath, QString()};
        auto *section = window.findChild<QGroupBox *>(QStringLiteral("mcpSection"));
        QVERIFY(section);
        auto *editor = section->findChild<QTextEdit *>(QStringLiteral("mcpConfigJson"));
        auto *button = section->findChild<QPushButton *>(QStringLiteral("copyMcpConfig"));
        QVERIFY(editor);
        QVERIFY(button);
        QVERIFY(editor->isReadOnly());
        const QJsonObject expected{{QStringLiteral("mcpServers"), QJsonObject{
            {QStringLiteral("kerf"), QJsonObject{
                {QStringLiteral("command"), mcpExecutable(projectPath)},
                {QStringLiteral("args"), QJsonArray{QStringLiteral("--dir"), projectPath}}
            }}}}};
        QCOMPARE(QJsonDocument::fromJson(editor->toPlainText().toUtf8()).object(), expected);
        button->click();
        QCOMPARE(QApplication::clipboard()->text(), editor->toPlainText());
        QCOMPARE(section->findChild<QLabel *>(QStringLiteral("muted"))->text(), QStringLiteral("MCP config copied"));
    }

    // The agent request carries the settings of the active connection, as
    // before; the model is the More menu's Agent settings… key.
    void agentPayloadFollowsSettingsProtocol() {
        MainWindow window{QString(), QString()};
        showAssistant(window);
        QVERIFY(!window.findChild<QComboBox *>(QStringLiteral("agentProvider")));
        // No model field: the More menu offers the models.
        QVERIFY(!window.findChild<QComboBox *>(QStringLiteral("agentModel")));
        QWidget *panel = window.findChild<QWidget *>(QStringLiteral("chatPanel"));
        QVERIFY(panel);
        QVERIFY(window.m_preferences->isAncestorOf(window.m_anthropicKey));
        for (auto *label : panel->findChildren<QLabel *>()) {
            QVERIFY(!label->text().contains(QStringLiteral("OpenAI")));
            QVERIFY(!label->text().contains(QStringLiteral("Anthropic")));
        }
        auto *anthropicRadio = window.findChild<QRadioButton *>(QStringLiteral("agentActiveAnthropic"));
        auto *openaiRadio = window.findChild<QRadioButton *>(QStringLiteral("agentActiveOpenai"));
        QVERIFY(anthropicRadio && openaiRadio);
        const QSignalBlocker anthropicBlock(anthropicRadio);
        const QSignalBlocker openaiBlock(openaiRadio);
        QVERIFY(window.m_advancedSettings->isAncestorOf(anthropicRadio));
        {
            const QSignalBlocker ownBlock(window.m_ownKeyRadio);
            window.m_ownKeyRadio->setChecked(true);
        }
        anthropicRadio->setChecked(true);
        // With no model saved, the one kerf-agent defaults to
        // (DEFAULT_ANTHROPIC_MODEL), not a retired one.
        if (!QSettings().contains(QStringLiteral("agent/anthropic/model")))
            QCOMPARE(window.m_anthropicModel->text(), QStringLiteral("claude-opus-5-5"));
        window.m_anthropicKey->setText(QStringLiteral("sk-ant-test"));
        window.m_anthropicBaseUrl->setText(QStringLiteral("https://anthropic.example/v1"));
        window.m_anthropicModel->setText(QStringLiteral("claude-test-model"));
        window.updateAgentModelChoices();
        const QJsonObject anthropicPayload = window.agentSettingsPayload();
        QCOMPARE(anthropicPayload.value(QStringLiteral("provider")).toString(), QStringLiteral("anthropic"));
        QCOMPARE(anthropicPayload.value(QStringLiteral("apiKey")).toString(), QStringLiteral("sk-ant-test"));
        QCOMPARE(anthropicPayload.value(QStringLiteral("model")).toString(), QStringLiteral("claude-test-model"));
        // The More menu names the model at Agent settings….
        QMenu *more = window.m_chat->buildMoreMenu();
        QAction *settings = actionNamed(more, QStringLiteral("Agent settings"));
        QVERIFY(settings);
        QCOMPARE(settings->property("kerfMenuKey").toString(), window.m_agentModel);
        QCOMPARE(window.m_agentModel, QStringLiteral("claude-test-model"));
        QCOMPARE(window.m_chat->model(), QStringLiteral("claude-test-model"));
        // Its Model submenu chooses the model the next request names.
        QMenu *models = nullptr;
        for (QAction *action : more->actions())
            if (action->menu() && action->text() == QStringLiteral("Model")) models = action->menu();
        QVERIFY(models);
        actionNamed(models, QStringLiteral("claude-sonnet-5-5"))->trigger();
        QCOMPARE(window.agentSettingsPayload().value(QStringLiteral("model")).toString(), QStringLiteral("claude-sonnet-5-5"));
        QCOMPARE(window.m_chat->model(), QStringLiteral("claude-sonnet-5-5"));
        delete more;
        openaiRadio->setChecked(true);
        window.m_openaiKey->setText(QStringLiteral("sk-openai-test"));
        window.m_openaiModel->setText(QStringLiteral("gpt-test-model"));
        window.updateAgentModelChoices();
        const QJsonObject openaiPayload = window.agentSettingsPayload();
        QCOMPARE(openaiPayload.value(QStringLiteral("provider")).toString(), QStringLiteral("openai"));
        QCOMPARE(openaiPayload.value(QStringLiteral("model")).toString(), QStringLiteral("gpt-test-model"));
        // Claude Code sends no key: it runs on the account it is signed in to.
        auto *claudeCode = window.findChild<QRadioButton *>(QStringLiteral("agentConnectionClaudeCode"));
        QVERIFY(claudeCode);
        {
            const QSignalBlocker claudeBlock(claudeCode);
            claudeCode->setChecked(true);
        }
        window.updateAgentModelChoices();
        QCOMPARE(window.aiAccess(), QStringLiteral("ready"));
        const QJsonObject claudePayload = window.agentSettingsPayload();
        QCOMPARE(claudePayload.value(QStringLiteral("provider")).toString(), QStringLiteral("claude-code"));
        QVERIFY(!claudePayload.contains(QStringLiteral("apiKey")));
        QCOMPARE(claudePayload.value(QStringLiteral("model")).toString(), QStringLiteral("default"));
    }

    // ---- the seed as the panel shows it ----------------------------------------

    // The mockup's seed: All 7 / Open 4 / Proposals 2; the threads in
    // timecode order under the welcome; each row's anatomy.
    void seedRendersTheMockup() {
        Seeded seed;
        ChatPanel &panel = seed.panel;
        QCOMPARE(panel.title()->text(), QStringLiteral("Conversation"));
        QCOMPARE(seed.model.counts().all, 7);
        QCOMPARE(seed.model.counts().open, 4);
        QCOMPARE(seed.model.counts().proposals, 2);
        QCOMPARE(seed.model.chip(), ReviewModel::Chip::All);
        QCOMPARE(threadIds(panel), (QStringList{QString::fromLatin1(ReviewModel::kWelcomeId), "c-0300", "c-0903", "c-1708", "c-2614",
                                                "c-2718", "c-2916"}));
        // Frame and header metrics.
        QCOMPARE(panel.findChild<QWidget *>(QStringLiteral("chatHeader"))->height(), ChatPanel::kHeaderHeight);
        // Comments and ⋯ only: search, sort and the filter are in its menu.
        QVERIFY(!panel.findChild<QWidget *>(QStringLiteral("chatChips")));
        QVERIFY(!panel.findChild<QWidget *>(QStringLiteral("chatClose")));
        QVERIFY(!panel.findChild<QWidget *>(QStringLiteral("chatResizeHandle")));
        QCOMPARE(panel.findChild<QWidget *>(QStringLiteral("chatHeader"))->findChildren<QAbstractButton *>().size(), 1);
        // The welcome: Kerf, agent, its words.
        CommentRow *welcome = panel.welcomeRow();
        QCOMPARE(welcome->name(), QStringLiteral("Kerf"));
        QCOMPARE(welcome->roleLabel()->text(), QStringLiteral("agent"));
        QVERIFY(welcome->text().startsWith(QStringLiteral("Tell me what to make")));
        // c-0300: Mina's settled note, its replies folded, Accepted in rev 20.
        CommentRow *road = panel.threadWidget("c-0300")->root();
        QCOMPARE(road->name(), QStringLiteral("Mina Park"));
        QCOMPARE(road->roleLabel()->text(), QStringLiteral("director"));
        QCOMPARE(road->avatar()->initials(), QStringLiteral("MP"));
        QVERIFY(road->avatar()->isRound());
        QCOMPARE(road->timecodeLink()->text(), QStringLiteral("00:00:03:00"));
        QVERIFY(road->ring()->isVisible() && road->ring()->isResolved());
        QCOMPARE(tagTexts(road), QStringList{QStringLiteral("Accepted in rev 20")});
        QCOMPARE(road->replyFold()->text(), QStringLiteral("2 replies"));
        QVERIFY(!road->replyFold()->isChecked());
        QVERIFY(!panel.threadWidget("c-0300")->repliesShown());
        // c-0903: open, #color, Kerf's reply with 3 steps and the colour card.
        CommentRow *sea = panel.threadWidget("c-0903")->root();
        QCOMPARE(tagTexts(sea), QStringList{QStringLiteral("#color")});
        QVERIFY(!sea->ring()->isResolved());
        CommentRow *kerfReply = panel.rowFor("c-0903-r1");
        QVERIFY(kerfReply && kerfReply->isReply());
        QCOMPARE(kerfReply->roleLabel()->text(), QStringLiteral("agent"));
        QCOMPARE(kerfReply->expander()->text(), QStringLiteral("3 steps"));
        QVERIFY(!kerfReply->repliesExpanded());
        kerfReply->expander()->click();
        QVERIFY(kerfReply->repliesExpanded());
        // c-1708: Kerf QC, the ranged TC, Failed and #captions, 1 folded reply with a drawing.
        CommentRow *caption = panel.threadWidget("c-1708")->root();
        QCOMPARE(caption->timecodeLink()->text(), QStringLiteral("00:00:17:08 · 2s12f"));
        QCOMPARE(tagTexts(caption), (QStringList{"Failed", "#captions"}));
        QCOMPARE(caption->tags().constFirst()->style(), TagButton::Style::Failed);
        QCOMPARE(caption->replyFold()->text(), QStringLiteral("1 reply"));
        QVERIFY(!panel.threadWidget("c-1708")->repliesShown());
        QVERIFY(!panel.rowFor("c-1708-r1")->drawingFlag()->isHidden());
        // c-2614: Blocking, the pinned card.
        CommentRow *logo = panel.threadWidget("c-2614")->root();
        QCOMPARE(tagTexts(logo), QStringList{QStringLiteral("Blocking")});
        QCOMPARE(logo->tags().constFirst()->style(), TagButton::Style::Blocking);
        QCOMPARE(ui::tipName(logo->tags().constFirst()), QStringLiteral("Blocks Render master"));
        // c-2718: QC (gauge), Failed, Blocking, #audio; your replies have no ring, Kerf's error.
        CommentRow *peak = panel.threadWidget("c-2718")->root();
        QCOMPARE(peak->avatar()->kind(), ui::Avatar::Kind::Qc);
        QCOMPARE(tagTexts(peak), (QStringList{"Failed", "Blocking", "#audio"}));
        QVERIFY(panel.threadWidget("c-2718")->repliesShown());
        CommentRow *yours = panel.rowFor("c-2718-r1");
        QCOMPARE(yours->name(), QStringLiteral("You"));
        QVERIFY(yours->avatar()->initials().isEmpty());
        QVERIFY(yours->ring()->isHidden());
        QVERIFY(yours->body()->text().contains(QStringLiteral("font-weight:500")));
        CommentRow *failed = panel.rowFor("c-2718-r2");
        QVERIFY(failed->isError() && !failed->errorLabel()->isHidden());
        QCOMPARE(failed->errorLabel()->text(), QStringLiteral("Couldn't edit"));
        QCOMPARE(failed->links().size(), 2);
        QCOMPARE(failed->links().at(0)->text(), QStringLiteral("Ask again"));
        // c-2916: Codex's settled note.
        CommentRow *endCard = panel.threadWidget("c-2916")->root();
        QCOMPARE(endCard->avatar()->kind(), ui::Avatar::Kind::Agent);
        QCOMPARE(tagTexts(endCard), QStringList{QStringLiteral("Accepted in rev 20")});
        // Inline timecodes are links that seek.
        CommentRow *transient = panel.rowFor("c-2614-r2");
        QVERIFY(transient->body()->text().contains(QStringLiteral("href=\"tc:26.48\"")));
        QSignalSpy seeks(&panel, &ChatPanel::seekRequested);
        emit transient->body()->linkActivated(QStringLiteral("tc:26.48"));
        QCOMPARE(seeks.size(), 1);
        QCOMPARE(seeks.constFirst().at(0).toDouble(), 26.48);
        road->timecodeLink()->click();
        QCOMPARE(seeks.last().at(0).toDouble(), 3.0);
        // Rows sit 16 in from the panel; replies 36 further; the bleed is 8.
        QCOMPARE(road->mapTo(&panel, QPoint()).x() + CommentRow::kBleedX, 16);
        QCOMPARE(kerfReply->mapTo(&panel, QPoint()).x() + CommentRow::kBleedX, 16 + 36);
        QCOMPARE(road->avatar()->width(), 24);
    }

    // The proposal cards: the pinned (blocking) one open with its header,
    // changes, versions and a primary Accept; Kerf's colour key closed with
    // a secondary Accept.
    void proposalCardsFollowTheSnapshot() {
        Seeded seed;
        ChatPanel &panel = seed.panel;
        ProposalCard *logo = panel.card("proposal-cc2");
        ProposalCard *sea = panel.card("proposal-kg");
        QVERIFY(logo && sea);
        QVERIFY(logo->isPinned() && !sea->isPinned());
        QCOMPARE(logo->pinHeaderText()->text(), QStringLiteral("Pinned proposal · blocking first"));
        QVERIFY(logo->isOpen() && !sea->isOpen());
        QCOMPARE(logo->titleLabel()->text(), QStringLiteral("Move logo reveal to the snare"));
        QCOMPARE(logo->changesLink()->text(), QStringLiteral("2 changes"));
        QCOMPARE(logo->changeRows().size(), 2);
        QCOMPARE(ui::tipName(logo->changeRows().at(0)), QStringLiteral("Logo_reveal · V4 → 00:00:25:20 −8f"));
        QVERIFY(logo->versionButton()->isVisible());
        QCOMPARE(logo->versionButton()->text(), QStringLiteral("v2"));
        QCOMPARE(logo->versions().size(), 2);
        QCOMPARE(logo->versions().at(0).label, QStringLiteral("v1 00:00:25:21 · −7 fr"));
        QCOMPARE(logo->acceptButton()->kind(), ui::Button::Kind::Primary);
        QCOMPARE(ui::tipName(logo->acceptButton()), QStringLiteral("Accept · resolves Mina's note"));
        QCOMPARE(ui::tipShortcut(logo->acceptButton()), ui::keyText(QKeySequence(Qt::CTRL | Qt::Key_Return)));
#ifdef Q_OS_MACOS
        QCOMPARE(ui::tipShortcut(logo->acceptButton()), QStringLiteral("⌘↩"));
#endif
        {
            // The tip names proposal.accept's key and follows it, except while
            // Accept is refused, when the tip is the reason.
            const QString tip = ui::tipName(logo->acceptButton());
            const QString key = ui::tipShortcut(logo->acceptButton());
            QAction accept;
            accept.setShortcut(Qt::Key_F9);
            panel.setAcceptAction(&accept);
            QCOMPARE(ui::tipShortcut(logo->acceptButton()), QKeySequence(Qt::Key_F9).toString(QKeySequence::NativeText));
            logo->setAcceptBlocked(QStringLiteral("Out of date"));
            accept.setShortcut(Qt::Key_F8);
            QCOMPARE(ui::tipName(logo->acceptButton()), QStringLiteral("Out of date"));
            QVERIFY(ui::tipShortcut(logo->acceptButton()).isEmpty());
            logo->setAcceptBlocked(QString());
            QCOMPARE(ui::tipName(logo->acceptButton()), tip);
            QCOMPARE(ui::tipShortcut(logo->acceptButton()), QKeySequence(Qt::Key_F8).toString(QKeySequence::NativeText));
            logo->setAcceptKey(key);
        }
        QCOMPARE(sea->acceptButton()->kind(), ui::Button::Kind::Secondary);
        // The card sits under Claude Code's first reply, across the thread.
        CommentRow *carrier = panel.rowFor("c-2614-r1");
        QVERIFY(carrier->isAncestorOf(logo));
        QCOMPARE(logo->mapTo(&panel, QPoint()).x(), 16 + 36);
        QVERIFY(!panel.rowFor("c-2614-r3")->isAncestorOf(logo));
        // The Script shows the document's diff, removals neg and additions pos.
        QVERIFY(!logo->diffView()->isVisible());
        logo->scriptButton()->click();
        QVERIFY(logo->diffView()->isVisible());
        QCOMPARE(logo->diffLines(), (QStringList{"- clip \"Logo_reveal\" V4 00:00:26:03", "+ clip \"Logo_reveal\" V4 00:00:25:20"}));
        QVERIFY(logo->diffView()->text().contains(tokens::color(tokens::neg).name()));
        QVERIFY(logo->diffView()->text().contains(tokens::color(tokens::pos).name()));
        // The sea card opens on its title.
        QTest::mouseClick(sea->titleLabel(), Qt::LeftButton);
        QVERIFY(sea->isOpen());
        QVERIFY(sea->hasColour());
        QCOMPARE(sea->colourTitle()->text(), QStringLiteral("B003_C011 · Huehue"));
        QCOMPARE(sea->colourParams()->text(), QStringLiteral("Hue −9, Sat −6"));
    }

    // Filters, sort and search: chips, From with the filter dot, tags,
    // Hide resolved; Newest shows its label; search narrows the list.
    void filtersSortAndSearch() {
        Seeded seed;
        ChatPanel &panel = seed.panel;
        const auto choose = [&panel, &seed](const char *name) {
            QMenu *more = panel.buildMoreMenu();
            const auto actions = more->findChildren<QAction *>();
            for (QAction *action : actions)
                if (action->objectName() == QLatin1String(name)) action->trigger();
            delete more;
            seed.settle();
        };
        choose("commentsShow.Open");
        QCOMPARE(seed.model.chip(), ReviewModel::Chip::Open);
        // A filter that hides rows shows on the ⋯ button.
        QVERIFY(panel.filterDotShown());
        QVERIFY(!threadIds(panel).contains("c-0300") && threadIds(panel).contains("c-1708"));
        choose("commentsShow.Proposals");
        QCOMPARE(recordIds(panel), (QStringList{"c-0903", "c-2614"}));
        // Its rows name their counts.
        {
            QMenu *more = panel.buildMoreMenu();
            QAction *open = nullptr;
            for (QAction *action : more->findChildren<QAction *>())
                if (action->objectName() == QLatin1String("commentsShow.Open")) open = action;
            QVERIFY(open);
            QCOMPARE(ui::menuKey(open), QStringLiteral("4"));
            delete more;
        }
        choose("commentsShow.All");
        // From Kerf, through the menu.
        QVERIFY(!panel.filterDotShown());
        QMenu *filter = panel.buildFilterMenu();
        actionNamed(filter, QStringLiteral("Kerf"))->trigger();
        delete filter;
        seed.settle();
        QVERIFY(panel.filterDotShown());
        QCOMPARE(recordIds(panel), (QStringList{"c-0903", "c-1708", "c-2718"}));
        filter = panel.buildFilterMenu();
        QVERIFY(actionNamed(filter, QStringLiteral("Kerf"))->isChecked());
        QVERIFY(actionNamed(filter, QStringLiteral("#audio")));
        QCOMPARE(actionNamed(filter, QStringLiteral("Blocking"))->property("kerfMenuKey").toString(), QStringLiteral("2"));
        actionNamed(filter, QStringLiteral("Kerf"))->trigger(); // again: anyone
        delete filter;
        seed.settle();
        QVERIFY(!panel.filterDotShown());
        // A tag clicked in a row toggles the same filter.
        panel.threadWidget("c-0903")->root()->tags().constFirst()->click();
        seed.settle();
        QCOMPARE(recordIds(panel), QStringList{"c-0903"});
        panel.threadWidget("c-0903")->root()->tags().constFirst()->click();
        seed.settle();
        QCOMPARE(threadIds(panel).size(), 7);
        // Hide resolved.
        filter = panel.buildFilterMenu();
        actionNamed(filter, QStringLiteral("Hide resolved"))->trigger();
        delete filter;
        seed.settle();
        QVERIFY(!threadIds(panel).contains("c-0300") && !threadIds(panel).contains("c-2916"));
        seed.model.clearFilters();
        seed.settle();
        // Sort: Newest.
        QMenu *sort = panel.buildSortMenu();
        QVERIFY(actionNamed(sort, QStringLiteral("Timecode"))->isChecked());
        actionNamed(sort, QStringLiteral("Newest"))->trigger();
        delete sort;
        seed.settle();
        QCOMPARE(threadIds(panel).at(1), QStringLiteral("c-2718"));
        seed.model.setSort(ReviewModel::Sort::Timecode);
        // Search (⌘F, and ⋯ ▸ Search comments).
        choose("commentsSearch");
        QVERIFY(panel.searchRow()->isVisible());
        panel.searchField()->setText(QStringLiteral("magenta"));
        seed.settle();
        QCOMPARE(recordIds(panel), QStringList{"c-0903"});
        QTest::keyClick(panel.searchField(), Qt::Key_Escape);
        seed.settle();
        QVERIFY(!panel.searchRow()->isVisible());
        QCOMPARE(threadIds(panel).size(), 7);
    }

    // The pin bar shows only while the pinned card's actions are out of
    // view, with its author, summary and N/M; its arrows page the open
    // proposals in pin order, and a click brings the card into view.
    void pinBarAppearsOnlyWhenTheActionsScrollAway() {
        ui::setReducedMotionOverride(1);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        Seeded seed;
        ChatPanel &panel = seed.panel;
        QScrollBar *bar = panel.scroll()->verticalScrollBar();
        ProposalCard *logo = panel.card("proposal-cc2");
        QVERIFY(bar->maximum() > 0);
        // Scrolled so the footer is in view: no bar.
        const int footer = logo->footer()->mapTo(panel.listContent(), QPoint()).y();
        bar->setValue(std::max(0, footer - 120));
        seed.settle();
        QVERIFY(!panel.pinBarWanted());
        QCOMPARE(panel.pinBar()->height(), 0);
        // At the top, the card is far below: the bar.
        bar->setValue(0);
        seed.settle();
        QVERIFY(panel.pinBarWanted());
        QCOMPARE(panel.pinBar()->height(), ChatPanel::kPinHeight);
        QCOMPARE(panel.pinAuthor()->text(), QStringLiteral("Claude Code"));
        QCOMPARE(panel.pinSummary()->text(), QStringLiteral("Move logo reveal to the snare"));
        QCOMPARE(panel.pinCount()->text(), QStringLiteral("1/2"));
        panel.pinNext()->click();
        QCOMPARE(panel.pinCount()->text(), QStringLiteral("2/2"));
        QCOMPARE(panel.pinAuthor()->text(), QStringLiteral("Kerf"));
        QCOMPARE(panel.pinBarProposal(), QStringLiteral("proposal-kg"));
        panel.pinPrevious()->click();
        QCOMPARE(panel.pinBarProposal(), QStringLiteral("proposal-cc2"));
        // A click on the bar scrolls the card's comment to the list's top + 24.
        QTest::mouseClick(panel.pinBar(), Qt::LeftButton, {}, QPoint(60, 16));
        seed.settle();
        const int carrier = panel.rowFor("c-2614-r1")->mapTo(panel.listContent(), QPoint()).y();
        QCOMPARE(bar->value(), std::min(bar->maximum(), carrier - 24));
    }

    // On first show the current thread (the one at the playhead) is at the
    // top, 12 under the chips.
    void firstShowPlacesTheCurrentThread() {
        Seeded seed;
        ChatPanel &panel = seed.panel;
        QVERIFY(panel.placed());
        QCOMPARE(seed.model.currentThread(), QStringLiteral("c-1708"));
        CommentRow *root = panel.threadWidget("c-1708")->root();
        QVERIFY(root->isCurrent());
        const int top = root->mapTo(panel.listContent(), QPoint()).y();
        QScrollBar *bar = panel.scroll()->verticalScrollBar();
        QVERIFY2(bar->value() >= std::min(bar->maximum(), top - ChatPanel::kListTop),
                 qPrintable(QStringLiteral("value %1, max %2, top %3").arg(bar->value()).arg(bar->maximum()).arg(top)));
        // Clicking another root makes it current.
        QTest::mouseClick(panel.threadWidget("c-0903")->root(), Qt::LeftButton, {}, QPoint(200, 8));
        QCOMPARE(seed.model.currentThread(), QStringLiteral("c-0903"));
        QVERIFY(panel.threadWidget("c-0903")->root()->isCurrent());
        QVERIFY(!panel.threadWidget("c-1708")->root()->isCurrent());
    }

    // Viewing as Mina: Accept and Reject give way to "Waiting for the editor".
    void guestModeHidesAcceptAndReject() {
        Seeded seed;
        seed.model.setViewAs(kMina);
        seed.settle();
        QVERIFY(seed.model.guestMode());
        ProposalCard *logo = seed.panel.card("proposal-cc2");
        QVERIFY(logo->acceptButton()->isHidden() && logo->rejectButton()->isHidden());
        QVERIFY(!logo->waitChip()->isHidden());
        QCOMPARE(logo->waitChip()->text(), QStringLiteral("Waiting for the editor"));
        QMenu *more = seed.panel.buildMoreMenu();
        QAction *viewAs = actionNamed(more, QStringLiteral("View as Mina Park"));
        QVERIFY(viewAs && viewAs->isChecked());
        delete more;
        seed.model.setViewAs(QString());
        seed.settle();
        QVERIFY(!seed.panel.card("proposal-cc2")->acceptButton()->isHidden());
    }

    // ---- wiring to kerfd ---------------------------------------------------------

    // The window feeds the model from the bus, sets it on the bus and asks
    // for the review log on connecting.
    // The row under the pointer and the row pressed reach the stage through
    // the bus: threadHovered (empty on leaving) and commentSelected.
    void hoverAndSelectionReachTheBus() {
        SeededWindow seeded;
        MainWindow &window = seeded.window;
        CommentRow *reply = window.m_chat->rowFor(QStringLiteral("c-2614-r1"));
        QVERIFY(reply && !reply->threadId().isEmpty());
        QSignalSpy hovered(window.m_bus, &EditorBus::threadHovered);
        QSignalSpy selected(window.m_bus, &EditorBus::commentSelected);
        QEnterEvent enter(QPointF(4, 4), QPointF(4, 4), reply->mapToGlobal(QPointF(4, 4)));
        QCoreApplication::sendEvent(reply, &enter);
        QCOMPARE(hovered.size(), 1);
        QCOMPARE(hovered.last().first().toString(), reply->threadId());
        QEvent leave(QEvent::Leave);
        QCoreApplication::sendEvent(reply, &leave);
        QCOMPARE(hovered.last().first().toString(), QString());
        QTest::mousePress(reply, Qt::LeftButton, {}, QPoint(4, 4));
        QCOMPARE(selected.size(), 1);
        QCOMPARE(selected.last().first().toString(), QStringLiteral("c-2614-r1"));
    }

    void windowWiresTheReviewModel() {
        SeededWindow seeded;
        MainWindow &window = seeded.window;
        QVERIFY(window.m_bus->review());
        QCOMPARE(window.m_bus->review(), window.m_reviewWiring->model());
        QVERIFY(!seeded.last(QStringLiteral("comment_list")).isEmpty());
        QCOMPARE(window.m_reviewWiring->model()->counts().all, 7);
        QCOMPARE(window.m_reviewWiring->model()->counts().open, 4);
        // ⌘↩ and Esc act on the pinned proposal.
        QCOMPARE(window.m_proposalId, QStringLiteral("proposal-cc2"));
        // bus openThread goes to the thread and flashes its root.
        emit window.m_bus->openThread(QStringLiteral("c-2718-r2"));
        seeded.settle();
        QCOMPARE(window.m_reviewWiring->model()->currentThread(), QStringLiteral("c-2718"));
        QVERIFY(window.m_chat->threadWidget("c-2718")->root()->isFlashing());
        // askPrefill opens the composer as a reply.
        emit window.m_bus->askPrefill(QStringLiteral("@Kerf bring true peak under −1 dBTP on the music bus"), QStringLiteral("c-2718"));
        seeded.settle();
        QCOMPARE(window.m_chat->replyTarget(), QStringLiteral("c-2718"));
        QCOMPARE(window.m_chatInput->toPlainText(), QStringLiteral("@Kerf bring true peak under −1 dBTP on the music bus"));
        QVERIFY(window.m_chat->replyBar()->isOpen());
        QVERIFY(window.m_chat->replyBar()->label()->text().contains(QStringLiteral("QC")));
        // An unlinked proposal's thread is not in the review log: no reply
        // target, so the post carries no parent kerfd would refuse.
        QJsonObject snapshot = seedSnapshot();
        QJsonArray proposals = snapshot.value("proposals").toArray();
        proposals.append(proposalEntry("proposal-un", "pending", agent("mcp", "Cursor", "cursor", "MCP"), 1,
                                       QJsonArray{versionEntry(1, "proposal-un", "trim end out -6f\n")}, "trim end out -6f\n",
                                       {{"staged", "2026-11-03T08:00:00Z"}}));
        snapshot.insert("proposals", proposals);
        window.m_reviewWiring->handleEvent(QStringLiteral("state"), QJsonObject{{"type", "state"}, {"snapshot", snapshot}});
        seeded.settle();
        const ReviewModel::Thread *unlinked = window.m_reviewWiring->model()->thread(QStringLiteral("proposal:proposal-un"));
        QVERIFY(unlinked && unlinked->origin == ReviewModel::Origin::Proposal);
        emit window.m_bus->askPrefill(QStringLiteral("@Mina is this trim right?"), QStringLiteral("proposal:proposal-un"));
        seeded.settle();
        QVERIFY(window.m_chat->replyTarget().isEmpty());
        QVERIFY(!window.m_chat->replyBar()->isOpen());
        QTest::keyClick(window.m_chatInput, Qt::Key_Return);
        const QJsonObject add = seeded.last(QStringLiteral("comment_add"));
        QCOMPARE(add.value("body").toString(), QStringLiteral("@Mina is this trim right?"));
        QVERIFY(!add.contains("parent"));
    }

    // Post: text for Kerf (@Kerf, or no mention) is an agent turn with its
    // comment; text for people only is comment_add. The stamp gives the
    // time unless detached or replying; a drawing goes with it.
    void postRoutesToKerfOrToTheReviewLog() {
        SeededWindow seeded;
        MainWindow &window = seeded.window;
        Composer *composer = window.m_chat->composerWidget();
        window.m_playhead = 17.32;
        window.m_chat->setPlayhead(17.32);
        QCOMPARE(composer->stamp()->text(), QStringLiteral("00:00:17:08"));
        // For Kerf, stamped.
        composer->fill(QStringLiteral("@Kerf tighten the caption timing"));
        QVERIFY(window.m_sendButton->isEnabled());
        QTest::keyClick(window.m_chatInput, Qt::Key_Return);
        QJsonObject agentTurn = seeded.last(QStringLiteral("agent"));
        QCOMPARE(agentTurn.value("text").toString(), QStringLiteral("@Kerf tighten the caption timing"));
        QCOMPARE(agentTurn.value("comment").toObject().value("at").toDouble(), 17.32);
        QVERIFY(!agentTurn.value("comment").toObject().contains("parent"));
        QCOMPARE(agentTurn.value("focus").toObject().value("fromFrame").toInt(), 433);
        QVERIFY(agentTurn.contains("provider") || agentTurn.contains("cloud") || agentTurn.contains("model"));
        QVERIFY(composer->text().isEmpty());
        // The turn's local rows: you, then Kerf drafting; Send gives way to Stop.
        seeded.settle();
        QList<CommentRow *> rows = window.m_chat->rows();
        QVERIFY(rows.size() >= 2);
        QCOMPARE(rows.at(rows.size() - 2)->name(), QStringLiteral("You"));
        // Stamped, not a reply: the local row carries its timecode.
        QVERIFY(!rows.at(rows.size() - 2)->timecodeLink()->isHidden());
        QCOMPARE(rows.at(rows.size() - 2)->timecodeLink()->text(), QStringLiteral("00:00:17:08"));
        QVERIFY(rows.constLast()->isDrafting());
        QCOMPARE(rows.constLast()->name(), QStringLiteral("Kerf is drafting"));
        QVERIFY(composer->stopButton()->isVisible() && !composer->sendButton()->isVisible());
        // The composer's Stop asks once, and the bridge gets its abort.
        QSignalSpy stops(window.m_chat, &ChatPanel::stopRequested);
        window.m_lastCommand = {};
        composer->stopButton()->click();
        QCOMPARE(stops.count(), 1);
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("abort"));
        // While Kerf drafts, posting to it waits.
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", false}});
        window.handleEvent(QJsonObject{{"type", "agent_state"}, {"state", "completed"}});
        seeded.settle();
        // For people only: comment_add with a client id, stamped.
        composer->fill(QStringLiteral("@Mina is the sky better now?"));
        QTest::keyClick(window.m_chatInput, Qt::Key_Return);
        QJsonObject add = seeded.last(QStringLiteral("comment_add"));
        QCOMPARE(add.value("body").toString(), QStringLiteral("@Mina is the sky better now?"));
        QVERIFY(add.value("id").toString().startsWith(QStringLiteral("c-qt-")));
        QCOMPARE(add.value("id").toString(), window.m_reviewWiring->lastClientId());
        QCOMPARE(add.value("at").toDouble(), 17.32);
        QVERIFY(!add.contains("parent"));
        seeded.settle();
        CommentRow *pending = window.m_chat->rowFor(add.value("id").toString());
        QVERIFY(pending);
        // The log has it now: the optimistic row goes.
        QJsonArray items = seedComments();
        items.append(note(qPrintable(add.value("id").toString()), you(), "2026-11-04T10:00:00.000000Z", "@Mina is the sky better now?",
                          {{"at", 17.32}}));
        window.m_reviewWiring->handleEvent(QStringLiteral("comments"), QJsonObject{{"type", "comments"}, {"items", items}});
        seeded.settle();
        QCOMPARE(window.m_chat->rows().count(pending), 0);
        // A refused add shows on the composer.
        composer->fill(QStringLiteral("@Tom one more"));
        QTest::keyClick(window.m_chatInput, Qt::Key_Return);
        const QString refused = window.m_reviewWiring->lastClientId();
        window.m_reviewWiring->handleEvent(QStringLiteral("error"), QJsonObject{{"type", "error"},
                                                                                {"message", "INVALID_COMMENT"},
                                                                                {"command", "comment_add"},
                                                                                {"id", refused}});
        QVERIFY(!composer->errorLabel()->isHidden());
        QVERIFY(window.m_chat->rowFor(refused)->isError());
        // A reply: parent, no time; the stamp detached: no time either.
        window.m_chat->setReplyTarget(QStringLiteral("c-2614"));
        composer->fill(QStringLiteral("@Mina two frames it is"));
        QTest::keyClick(window.m_chatInput, Qt::Key_Return);
        add = seeded.last(QStringLiteral("comment_add"));
        QCOMPARE(add.value("parent").toString(), QStringLiteral("c-2614"));
        QVERIFY(!add.contains("at"));
        QVERIFY(window.m_chat->replyTarget().isEmpty());
        composer->stamp()->click();
        QVERIFY(!composer->stamped());
        composer->fill(QStringLiteral("@Tom check the grade"));
        QTest::keyClick(window.m_chatInput, Qt::Key_Return);
        QVERIFY(!seeded.last(QStringLiteral("comment_add")).contains("at"));
        composer->stamp()->click();
        QVERIFY(composer->stamped());
        // Draw on frame: the stage takes the drawing, which goes with the note.
        QSignalSpy draw(window.m_bus, &EditorBus::drawingRequested);
        composer->drawButton()->click();
        QCOMPARE(draw.size(), 1);
        QCOMPARE(composer->drawState(), Composer::Draw::Drawing);
        const QJsonArray strokes{QJsonObject{{"tool", "ellipse"}, {"points", QJsonArray{QJsonArray{0.1, 0.2}, QJsonArray{0.3, 0.4}}}}};
        emit window.m_bus->drawingFinished(strokes);
        QCOMPARE(composer->drawState(), Composer::Draw::Attached);
        composer->fill(QStringLiteral("@Tom here"));
        QTest::keyClick(window.m_chatInput, Qt::Key_Return);
        QCOMPARE(seeded.last(QStringLiteral("comment_add")).value("drawing").toArray(), strokes);
        QCOMPARE(composer->drawState(), Composer::Draw::None);
        // A reply to Kerf's thread with no mention is for Kerf, about its range.
        window.m_chat->setReplyTarget(QStringLiteral("c-1708"));
        composer->fill(QStringLiteral("move it up a little"));
        QTest::keyClick(window.m_chatInput, Qt::Key_Return);
        agentTurn = seeded.last(QStringLiteral("agent"));
        QCOMPARE(agentTurn.value("text").toString(), QStringLiteral("move it up a little"));
        QCOMPARE(agentTurn.value("comment").toObject().value("parent").toString(), QStringLiteral("c-1708"));
        QVERIFY(!agentTurn.value("comment").toObject().contains("at"));
        QCOMPARE(agentTurn.value("focus").toObject().value("scope").toString(), QStringLiteral("range"));
        QCOMPARE(agentTurn.value("focus").toObject().value("fromFrame").toInt(), 433);
        QCOMPARE(agentTurn.value("focus").toObject().value("toFrame").toInt(), 495);
    }

    // The agent stream lands in the turn's drafting row: its latest status
    // as meta, tools as steps, text as it streams; Stop aborts and leaves
    // "Kerf · stopped" with Ask again; the review log then takes over.
    void draftingRowFollowsTheAgentStream() {
        SeededWindow seeded;
        MainWindow &window = seeded.window;
        Composer *composer = window.m_chat->composerWidget();
        window.m_chat->setReplyTarget(QStringLiteral("c-2718"));
        composer->fill(QStringLiteral("@Kerf limit the music bus here, again"));
        QTest::keyClick(window.m_chatInput, Qt::Key_Return);
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", true}});
        window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "status"}, {"text", "limiter on A2 Music"}});
        window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "tool"}, {"text", "Reading the A2 Music bus"}});
        seeded.settle();
        CommentRow *drafting = window.m_chat->turnKerfRow(false);
        QVERIFY(drafting && drafting->isDrafting());
        QVERIFY(window.m_chat->threadWidget("c-2718")->replyRows().contains(drafting));
        QCOMPARE(drafting->draftMeta(), QStringLiteral("Reading the A2 Music bus"));
        QCOMPARE(drafting->replies(), QStringList{QStringLiteral("Reading the A2 Music bus")});
        QVERIFY(!drafting->dots()->isHidden());
        QVERIFY(!drafting->skeleton()->parentWidget()->isHidden());
        window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "delta"}, {"text", "Adding a limiter"}});
        QCOMPARE(drafting->text(), QStringLiteral("Adding a limiter"));
        QVERIFY(drafting->skeleton()->parentWidget()->isHidden());
        // Stop: the bridge's abort, then "Kerf · stopped" with Ask again.
        drafting->stopButton()->click();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("abort"));
        window.handleEvent(QJsonObject{{"type", "agent_state"}, {"state", "aborted"}});
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", false}});
        seeded.settle();
        QVERIFY(drafting->isStopped());
        QCOMPARE(drafting->name(), QStringLiteral("Kerf · stopped"));
        QCOMPARE(drafting->stopButton()->text(), QStringLiteral("Ask again"));
        QVERIFY(composer->sendButton()->isVisible());
        drafting->stopButton()->click();
        const QJsonObject again = seeded.last(QStringLiteral("agent"));
        QCOMPARE(again.value("text").toString(), QStringLiteral("@Kerf limit the music bus here, again"));
        QCOMPARE(again.value("comment").toObject().value("parent").toString(), QStringLiteral("c-2718"));
        window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "done"}, {"text", "Added a limiter on A2 Music."}});
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", false}});
        seeded.settle();
        QVERIFY(window.m_chat->transcript().contains(QStringLiteral("Added a limiter on A2 Music.")));
        // kerfd records the turn: your request and Kerf's reply replace the local rows.
        QJsonArray items = seedComments();
        const QString now = QDateTime::currentDateTimeUtc().toString(Qt::ISODateWithMs);
        // Both requests (the stopped one and the one asked again) are kept.
        items.append(reply("c-2718-r4a", "c-2718", you(), qPrintable(now), "@Kerf limit the music bus here, again"));
        items.append(reply("c-2718-r4", "c-2718", you(), qPrintable(now), "@Kerf limit the music bus here, again"));
        items.append(reply("c-2718-r5", "c-2718", kerfAuthor(), qPrintable(now), "Added a limiter on A2 Music.",
                           {{"steps", QJsonArray{"Reading the A2 Music bus"}}}));
        window.m_reviewWiring->handleEvent(QStringLiteral("comments"), QJsonObject{{"type", "comments"}, {"items", items}});
        seeded.settle();
        seeded.settle();
        int answers = 0;
        for (CommentRow *row : window.m_chat->rows())
            if (row->text() == QStringLiteral("Added a limiter on A2 Music.")) ++answers;
        QCOMPARE(answers, 1);
        QVERIFY(window.m_chat->rowFor("c-2718-r5"));
        QCOMPARE(window.m_chat->rowFor("c-2718-r5")->expander()->text(), QStringLiteral("1 step"));
    }

    // Accept and Reject send the version shown with the revision; the
    // versions menu shows an older version, previews it, and Accept then
    // sends its id. The card settles to one line once kerfd accepts.
    void versionsMenuAcceptsTheChosenVersion() {
        SeededWindow seeded;
        MainWindow &window = seeded.window;
        ProposalCard *logo = window.m_chat->card("proposal-cc2");
        QVERIFY(logo);
        logo->showVersionMenu();
        QMenu *menu = nullptr;
        QTRY_VERIFY((menu = openMenu(QStringLiteral("versionsMenu"))));
        QCOMPARE(menu->actions().size(), 2);
        QVERIFY(menu->actions().at(1)->isChecked());
        menu->actions().at(0)->trigger();
        menu->close();
        seeded.settle();
        const QJsonObject preview = seeded.last(QStringLiteral("preview"));
        QCOMPARE(preview.value("proposalId").toString(), QStringLiteral("proposal-cc1"));
        QVERIFY(preview.contains("start") && preview.contains("duration"));
        logo = window.m_chat->card("proposal-cc2");
        QCOMPARE(logo->shownId(), QStringLiteral("proposal-cc1"));
        QCOMPARE(logo->versionButton()->text(), QStringLiteral("v1"));
        // ⌘↩ (proposal.accept) accepts the version on screen, not the latest.
        QCOMPARE(window.m_proposalId, QStringLiteral("proposal-cc1"));
        window.m_lastCommand = {};
        window.triggerCommand(QStringLiteral("proposal.accept"));
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("accept"));
        QCOMPARE(window.m_lastCommand.value("proposalId").toString(), QStringLiteral("proposal-cc1"));
        seeded.sent.clear();
        logo->acceptButton()->click();
        QJsonObject accept = seeded.last(QStringLiteral("accept"));
        QCOMPARE(accept.value("proposalId").toString(), QStringLiteral("proposal-cc1"));
        QCOMPARE(accept.value("revision").toInteger(), qint64(22));
        // kerfd accepted v1 at rev 23.
        QJsonObject snapshot = seedSnapshot();
        QJsonArray proposals = snapshot.value("proposals").toArray();
        QJsonObject cc = proposals.at(0).toObject();
        cc.insert("status", "accepted");
        cc.insert("proposalId", "proposal-cc1");
        cc.insert("appliedRevision", 23);
        proposals.replace(0, cc);
        snapshot.insert("proposals", proposals);
        snapshot.insert("revision", 23);
        window.m_reviewWiring->handleEvent(QStringLiteral("state"), QJsonObject{{"type", "state"}, {"snapshot", snapshot}});
        seeded.settle();
        ProposalCard *settled = window.m_chat->card("proposal-cc1");
        QVERIFY(settled && settled->isSettled());
        QVERIFY(settled->settledLabel()->text().startsWith(QStringLiteral("Accepted in rev 23 · You · ")));
        // The pin moved to Kerf's colour key.
        QCOMPARE(window.m_proposalId, QStringLiteral("proposal-kg"));
        ProposalCard *sea = window.m_chat->card("proposal-kg");
        QVERIFY(sea->isPinned());
        sea->rejectButton()->setVisible(true);
        sea->setOpen(true);
        sea->rejectButton()->click();
        QCOMPARE(seeded.last(QStringLiteral("reject")).value("proposalId").toString(), QStringLiteral("proposal-kg"));
    }

    // A colour change on one clip has a line of its own on the card, and
    // asks the engine for no frames.
    void aColourChangeHasItsOwnLine() {
        SeededWindow seeded;
        MainWindow &window = seeded.window;
        ProposalCard *sea = window.m_chat->card("proposal-kg");
        QVERIFY(sea && sea->hasColour());
        QCOMPARE(sea->colour().clip, QStringLiteral("sea"));
        QCOMPARE(sea->colour().fx, QStringLiteral("color.huehue"));
        QVERIFY(!sea->colourTitle()->text().isEmpty());
        sea->changesLink()->click();
        seeded.settle();
        QVERIFY(seeded.last(QStringLiteral("effect_previews")).isEmpty());
    }

    // The Preview checkbox renders the proposal's range; off plays the program again.
    void previewCheckboxSendsPreview() {
        SeededWindow seeded;
        ProposalCard *logo = seeded.window.m_chat->card("proposal-cc2");
        logo->previewBox()->click();
        const QJsonObject preview = seeded.last(QStringLiteral("preview"));
        QCOMPARE(preview.value("proposalId").toString(), QStringLiteral("proposal-cc2"));
        QVERIFY(preview.value("duration").toDouble() > 0 && preview.value("duration").toDouble() <= 120);
        seeded.window.m_reviewWiring->handleEvent(QStringLiteral("preview"),
                                                  QJsonObject{{"type", "preview"}, {"status", "running"}, {"proposalId", "proposal-cc2"}});
        QCOMPARE(seeded.window.m_chat->previewProposal(), QStringLiteral("proposal-cc2"));
        logo->previewBox()->click();
        QCOMPARE(seeded.sent.constLast().value("type").toString(), QStringLiteral("preview_exit"));
        seeded.window.m_reviewWiring->handleEvent(QStringLiteral("preview"), QJsonObject{{"type", "preview"}, {"status", "exited"}});
        QVERIFY(seeded.window.m_chat->previewProposal().isEmpty());
    }

    // The ring resolves; reopening a thread an accept settled reopens the
    // comments carrying the proposal (the root was never resolved).
    void resolveRingSendsCommentSet() {
        SeededWindow seeded;
        MainWindow &window = seeded.window;
        window.m_chat->threadWidget("c-0903")->root()->ring()->click();
        QJsonObject set = seeded.last(QStringLiteral("comment_set"));
        QCOMPARE(set.value("id").toString(), QStringLiteral("c-0903"));
        QCOMPARE(set.value("resolved").toBool(), true);
        seeded.sent.clear();
        window.m_chat->threadWidget("c-0300")->root()->ring()->click();
        QCOMPARE(seeded.sent.size(), 1);
        QCOMPARE(seeded.sent.constFirst().value("id").toString(), QStringLiteral("c-0300-r1"));
        QCOMPARE(seeded.sent.constFirst().value("resolved").toBool(), false);
        // Your own replies have no Resolve; others' roots do.
        QVERIFY(window.m_chat->rowFor("c-2718-r1")->resolveAction()->isHidden());
        QVERIFY(!window.m_chat->threadWidget("c-0903")->root()->resolveAction()->isHidden());
    }

    // The row menu: Copy and the review link for all; Convert to sequence
    // marker for people's notes; Ask again, Edit and Delete on your own.
    void rowMenuOffersEachRowsActions() {
        SeededWindow seeded;
        MainWindow &window = seeded.window;
        // A link names the folder (what a closed project is found by), not
        // the document's name.
        window.m_projectName = QStringLiteral("Harbor Teaser");
        window.m_projectDirectory = QDir(QDir::tempPath()).filePath(QStringLiteral("atlas"));
        QMenu *menu = window.m_chat->buildRowMenu(QStringLiteral("c-2614"));
        QVERIFY(actionNamed(menu, QStringLiteral("Copy")));
        QAction *link = actionNamed(menu, QStringLiteral("Copy link"));
        QCOMPARE(link->text(), QStringLiteral("Copy link kerf://atlas/rev22@00:00:26:14"));
        QAction *marker = actionNamed(menu, QStringLiteral("Convert to sequence marker"));
        QVERIFY(marker && !actionNamed(menu, QStringLiteral("Delete")));
        marker->trigger();
        const QJsonArray ops = window.m_lastCommand.value("ops").toArray();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("edit"));
        QCOMPARE(ops.at(0).toObject().value("op").toString(), QStringLiteral("add-marker"));
        QCOMPARE(ops.at(0).toObject().value("at").toInt(), 664);
        QCOMPARE(ops.at(0).toObject().value("note").toString(), QStringLiteral("Logo lands late, should hit on the snare"));
        delete menu;
        menu = window.m_chat->buildRowMenu(QStringLiteral("c-2718-r3"));
        QVERIFY(actionNamed(menu, QStringLiteral("Ask again")) && actionNamed(menu, QStringLiteral("Edit")));
        actionNamed(menu, QStringLiteral("Delete"))->trigger();
        QCOMPARE(seeded.last(QStringLiteral("comment_set")).value("deleted").toBool(), true);
        QCOMPARE(seeded.last(QStringLiteral("comment_set")).value("id").toString(), QStringLiteral("c-2718-r3"));
        actionNamed(menu, QStringLiteral("Edit"))->trigger();
        delete menu;
        QCOMPARE(window.m_chatInput->toPlainText(), QStringLiteral("Unlocked A2, try again"));
        window.m_chatInput->setPlainText(QStringLiteral("Unlocked A2 and A3, try again"));
        QTest::keyClick(window.m_chatInput, Qt::Key_Return);
        const QJsonObject edit = seeded.last(QStringLiteral("comment_set"));
        QCOMPARE(edit.value("id").toString(), QStringLiteral("c-2718-r3"));
        QCOMPARE(edit.value("body").toString(), QStringLiteral("Unlocked A2 and A3, try again"));
        // The More menu's export and review link.
        QMenu *more = window.m_chat->buildMoreMenu();
        QCOMPARE(actionNamed(more, QStringLiteral("Copy review link"))->text(), QStringLiteral("Copy review link kerf://atlas/rev22"));
        actionNamed(more, QStringLiteral("As sequence markers"))->trigger();
        QCOMPARE(seeded.last(QStringLiteral("export_comments")).value("format").toString(), QStringLiteral("markers"));
        delete more;
    }

    // Kerf's next steps are the welcome's Try asking and the composer's
    // suggestions, with each person's open notes, the failing checks and QC.
    void nextStepsBecomeSuggestions() {
        SeededWindow seeded;
        MainWindow &window = seeded.window;
        window.handleEvent(QJsonObject{{"type", "agent_next"},
                                       {"steps", QJsonArray{QJsonObject{{"label", "Duck the music"}, {"request", "Duck the music 6 dB under the voice-over"}}}}});
        window.m_reviewWiring->handleEvent(QStringLiteral("check"), QJsonObject{{"type", "check"}, {"status", "done"},
                                                                               {"summary", QJsonObject{{"rows", QJsonArray{
                                                                                   QJsonObject{{"name", "Caption overlap"}, {"passed", false}, {"at", 17.32}},
                                                                                   QJsonObject{{"name", "Gamut Rec.709"}, {"passed", true}}}}}}});
        seeded.settle();
        const QList<SuggestionButton *> welcome = window.m_chat->welcomeRow()->suggestions();
        QCOMPARE(welcome.size(), 1);
        QCOMPARE(welcome.constFirst()->text(), QStringLiteral("Duck the music"));
        welcome.constFirst()->click();
        QCOMPARE(window.m_chatInput->toPlainText(), QStringLiteral("Duck the music 6 dB under the voice-over"));
        QCOMPARE(window.m_lastCommand.value("type").toString() == QStringLiteral("agent"), false);
        QStringList labels;
        for (const ChatPanel::Suggestion &s : window.m_chat->suggestions()) labels << s.label;
        // People in the order their open notes come on the program.
        QCOMPARE(labels, (QStringList{"Duck the music", "Address Tom's open notes", "Address Mina's open notes", "Fix the caption overlap",
                                      "Run QC on the whole sequence"}));
        // The popover shows while the input is focused and empty.
        window.m_chatInput->clear();
        window.m_chatInput->setFocus();
        QTRY_VERIFY(window.m_chat->composerWidget()->suggestionsPopover()->isVisible());
        emit window.m_chat->composerWidget()->suggestionsPopover()->chosen(int(labels.size()) - 1);
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("check"));
        emit window.m_chat->composerWidget()->suggestionsPopover()->chosen(2);
        QVERIFY(window.m_chatInput->toPlainText().startsWith(QStringLiteral("@Kerf Address Mina Park's open notes:")));
        QVERIFY(window.m_chatInput->toPlainText().contains(QStringLiteral("00:00:26:14 Logo lands late")));
    }

    // @ completes Kerf, the agents seen and the people.
    void mentionCompletes() {
        Seeded seed;
        Composer *composer = seed.panel.composerWidget();
        // Kerf, the agents in the order they come, then the people by name.
        QCOMPARE(seed.panel.mentionCandidates(), (QStringList{"Kerf", "Cursor", "Claude Code", "Codex", "Mina Park", "Tom Lind"}));
        composer->input()->setFocus();
        QTRY_VERIFY(composer->input()->hasFocus());
        composer->mentionButton()->click();
        QVERIFY(composer->mentionList()->isVisible());
        QTest::keyClicks(composer->input(), "Mi");
        QCOMPARE(composer->mentionList()->items(), QStringList{QStringLiteral("Mina Park")});
        QTest::keyClick(composer->input(), Qt::Key_Return);
        QCOMPARE(composer->text(), QStringLiteral("@Mina "));
        QVERIFY(!composer->mentionList()->isVisible());
    }

    // The composer: Return sends, Shift-Return is a new line, nothing is
    // sent while offline.
    void composerKeysAndStates() {
        MainWindow window{QString(), QString()};
        window.show();
        showAssistant(window);
        window.m_chatInput->setEnabled(true);
        window.m_chatInput->setFocus();
        window.m_chatInput->setPlainText(QStringLiteral("First edit"));
        window.m_chatInput->moveCursor(QTextCursor::End);
        QTest::keyClick(window.m_chatInput, Qt::Key_Return, Qt::ShiftModifier);
        QTest::keyClicks(window.m_chatInput, "Second edit");
        QCOMPARE(window.m_chatInput->toPlainText(), QStringLiteral("First edit\nSecond edit"));
        QTest::keyClick(window.m_chatInput, Qt::Key_Return);
        QCOMPARE(window.m_chatInput->toPlainText(), QStringLiteral("First edit\nSecond edit"));
        QVERIFY(!window.m_sendButton->isEnabled());
        QVERIFY(window.m_abortButton->isHidden());
        QCOMPARE(window.m_chat->composer()->objectName(), QStringLiteral("chatComposer"));
        QCOMPARE(window.m_chat->composer()->parentWidget(), window.m_chat);
        QCOMPARE(window.m_chat->composer()->geometry().bottom(), window.m_chat->rect().bottom());
        // The send button: idle on, ready accent.
        auto *send = static_cast<SendButton *>(window.m_sendButton);
        QCOMPARE(send->background(), tokens::color(tokens::on));
        window.setConnection(QStringLiteral("ready"));
        QVERIFY(window.m_sendButton->isEnabled());
        QCOMPARE(send->background(), tokens::color(tokens::accent));
        // An agent that can take requests reads the design's placeholder.
        window.m_cloudAccess = QStringLiteral("ready");
        window.updateCommandStates();
        QCOMPARE(window.m_chatInput->placeholderText(), QStringLiteral("Ask Kerf, or leave a note"));
    }

    // Old callers of the conversation: appendConversation and the agent
    // events still land in rows (local until the log records them).
    void conversationCallsLandInRows() {
        MainWindow window{QString(), QString()};
        window.resize(1440, 900);
        window.show();
        showAssistant(window);
        window.appendConversationNote(QStringLiteral("Tool"), QStringLiteral("Inspecting timeline"));
        QVERIFY(window.m_chat->isEmpty());
        window.appendConversation(QStringLiteral("You"), QStringLiteral("Trim <intro>"));
        window.appendConversation(QStringLiteral("Kerf"), QStringLiteral("I’ll trim the opening."));
        window.appendConversationNote(QStringLiteral("Tool"), QStringLiteral("Trimmed v1b"));
        window.appendConversation(QStringLiteral("Error"), QStringLiteral("Could not read <media>"));
        const QList<CommentRow *> rows = window.m_chat->rows();
        QCOMPARE(rows.size(), 3);
        QCOMPARE(rows.at(0)->name(), QStringLiteral("You"));
        QCOMPARE(rows.at(1)->name(), QStringLiteral("Kerf"));
        // A step before the turn began belongs to no reply.
        QCOMPARE(rows.at(1)->replies(), QStringList{QStringLiteral("Trimmed v1b")});
        QCOMPARE(rows.at(1)->expander()->text(), QStringLiteral("1 step"));
        QVERIFY(rows.at(2)->isError());
        // Text comes literally, never as markup.
        QVERIFY(rows.at(0)->body()->text().contains(QStringLiteral("Trim &lt;intro&gt;")));
        QVERIFY(window.m_chat->transcript().contains(QStringLiteral("Could not read <media>")));
        // A done with no turn is a Kerf row of its own.
        window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "done"}, {"text", "Trimmed the intro."}});
        QVERIFY(window.m_chat->transcript().contains(QStringLiteral("Trimmed the intro.")));
    }

    // The Ask popover (#mAsk): 380 wide, the title, the Scope ghost with the
    // clip, the 56 pt field, a primary Ask; it sends the agent request with
    // the selection as focus, kept as a comment on the clip's range.
    void askingCutSendsTheSelectionAsFocus() {
        MainWindow window{QString(), QString()};
        QList<QJsonObject> sent;
        window.m_bus->setSender([&](const QJsonObject &command) {
            sent << command;
            window.m_lastCommand = command;
            return true;
        });
        window.show();
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 3}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24}, {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "c1"}, {"name", "Opening"}, {"timelineStart", 1.0}, {"duration", 2.0},
                            {"startFrame", 24}, {"durationFrames", 48}}}}}}}}}});
        window.selectClip(QStringLiteral("c1"));
        {
            const QSignalBlocker ownBlock(window.m_ownKeyRadio);
            window.m_ownKeyRadio->setChecked(true);
        }
        window.m_anthropicActiveRadio->setChecked(true);
        window.m_anthropicKey->setText(QStringLiteral("sk-ant-test"));
        window.updateCommandStates();
        window.m_commands->action(QStringLiteral("ai.ask"))->trigger();
        QTRY_VERIFY(window.m_askPrompt->isVisible());
        AskPrompt *ask = window.m_askPrompt;
        QCOMPARE(ask->cardRect().width(), AskPrompt::kWidth);
        QVERIFY(ask->titleLabel()->text().startsWith(QStringLiteral("Sequence at ")));
        QCOMPARE(ask->scope(), QStringLiteral("clip"));
        QVERIFY(ask->scopeButton()->text().startsWith(QStringLiteral("This clip · ")));
        QVERIFY(ask->scopeButton()->text().contains(QStringLiteral("Opening")));
        QCOMPARE(ask->input()->placeholderText(), QStringLiteral("Tell Kerf what to change"));
        QCOMPARE(ask->input()->parentWidget()->height(), 56);
        QCOMPARE(ask->askButton()->kind(), ui::Button::Kind::Primary);
        ask->input()->setPlainText(QStringLiteral("Tighten the start"));
        QTest::keyClick(ask->input(), Qt::Key_Return);
        QVERIFY(!ask->isVisible());
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("agent"));
        QCOMPARE(window.m_lastCommand.value("text").toString(), QStringLiteral("Tighten the start"));
        const QJsonObject focus = window.m_lastCommand.value("focus").toObject();
        QCOMPARE(focus.value("scope").toString(), QStringLiteral("clip"));
        QCOMPARE(focus.value("clip").toString(), QStringLiteral("c1"));
        QCOMPARE(focus.value("fromFrame").toInt(), 24);
        QCOMPARE(focus.value("toFrame").toInt(), 72);
        const QJsonObject comment = window.m_lastCommand.value("comment").toObject();
        QCOMPARE(comment.value("at").toDouble(), 1.0);
        QCOMPARE(comment.value("duration").toDouble(), 2.0);
        // While Kerf works, Stop is the registry's.
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", true}});
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("ai.stop")));
    }

    // With KERF_SCREENSHOT_DIR set, the panel's states are saved: the seed
    // at the mockup's start, guest mode, a filter, and Kerf drafting.
    void panelStatesScreenshots() {
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        if (shots.isEmpty()) QSKIP("KERF_SCREENSHOT_DIR is not set");
        const auto save = [&](QWidget &widget, const QString &name) {
            QCoreApplication::processEvents();
            QVERIFY(widget.grab().save(QDir(shots).filePath(QStringLiteral("comments-%1.png").arg(name))));
        };
        {
            Seeded seed;
            save(seed.panel, QStringLiteral("seed"));
            seed.panel.scroll()->verticalScrollBar()->setValue(0);
            seed.settle();
            save(seed.panel, QStringLiteral("top-pinbar"));
            seed.model.setViewAs(kMina);
            seed.settle();
            seed.panel.openThread(QStringLiteral("c-2614"), false);
            seed.settle();
            save(seed.panel, QStringLiteral("guest"));
        }
        {
            Seeded seed(480, 548);
            seed.model.setChip(ReviewModel::Chip::Open);
            seed.settle();
            save(seed.panel, QStringLiteral("open-480"));
        }
        {
            SeededWindow seeded;
            seeded.window.m_chat->setReplyTarget(QStringLiteral("c-2718"));
            seeded.window.m_chat->composerWidget()->fill(QStringLiteral("@Kerf limit the music bus here"));
            QTest::keyClick(seeded.window.m_chatInput, Qt::Key_Return);
            seeded.window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "status"}, {"text", "limiter on A2 Music"}});
            seeded.settle();
            seeded.window.m_chat->openThread(QStringLiteral("c-2718"), false);
            seeded.settle();
            save(*seeded.window.m_chat, QStringLiteral("drafting"));
            save(seeded.window, QStringLiteral("window"));
        }
    }

    // A thread the review gives the same widgets as before keeps them; one that
    // changed, or is new, is made again. Its text is measured once per width.
    void aThreadThatStaysTheSameKeepsItsWidgets() {
        SeededWindow seeded;
        MainWindow &window = seeded.window;
        ThreadWidget *road = window.m_chat->threadWidget(QStringLiteral("c-0300"));
        ThreadWidget *loudness = window.m_chat->threadWidget(QStringLiteral("c-2718"));
        QVERIFY(road && loudness);
        CommentRow *roadRow = road->root();
        QJsonArray items = seedComments();
        items.append(reply("c-2718-new", "c-2718", you(), "2026-11-03T10:30:00.000000Z", "One more thing on the bus"));
        items.append(note("c-new", you(), "2026-11-04T08:00:00.000000Z", "Start on the harbor instead"));
        window.m_reviewWiring->handleEvent(QStringLiteral("comments"), QJsonObject{{"type", "comments"}, {"items", items}});
        seeded.settle();
        // Untouched: the same widgets, in the list where they were.
        QCOMPARE(window.m_chat->threadWidget(QStringLiteral("c-0300")), road);
        QCOMPARE(road->root(), roadRow);
        QVERIFY(road->isVisible());
        // A reply came: made again, with it.
        QVERIFY(window.m_chat->threadWidget(QStringLiteral("c-2718")) != loudness);
        QVERIFY(window.m_chat->rowFor(QStringLiteral("c-2718-new")));
        // A new one is there, and the list is in the review's order, also in the layout.
        QVERIFY(window.m_chat->threadWidget(QStringLiteral("c-new")));
        QStringList order;
        for (const ReviewModel::Thread &thread : window.m_reviewWiring->model()->threads())
            if (thread.origin != ReviewModel::Origin::Welcome) order << thread.id;
        QStringList shown;
        for (ThreadWidget *thread : window.m_chat->threadWidgets())
            if (order.contains(thread->threadId())) shown << thread->threadId();
        QCOMPARE(shown, order);
        QVBoxLayout *list = window.m_chat->m_listLayout;
        for (qsizetype index = 0; index < window.m_chat->threadWidgets().size(); ++index)
            QCOMPARE(list->itemAt(int(index))->widget(), window.m_chat->threadWidgets().at(index));
        // Made again from nothing new, nothing is made again.
        const QList<ThreadWidget *> before = window.m_chat->threadWidgets();
        window.m_chat->rebuild();
        for (qsizetype index = 1; index < before.size(); ++index) QCOMPARE(window.m_chat->threadWidgets().at(index), before.at(index));
        // What the panel keeps of a thread, folded here, makes it again.
        emit road->root()->foldToggled(true);
        window.m_chat->rebuild();
        QVERIFY(window.m_chat->threadWidget(QStringLiteral("c-0300")) != road);

        ui::WrapLabel label;
        label.setFont(tokens::font(tokens::reading));
        label.setText(QStringLiteral("A sentence long enough to wrap onto a second line at this width."));
        const int two = label.heightForWidth(160);
        QCOMPARE(label.heightForWidth(160), two);
        QVERIFY(label.heightForWidth(1000) < two);
        label.setText(QStringLiteral("Short"));
        QVERIFY(label.heightForWidth(160) < two);
    }

    // A word as wide as the conversation, such as a file's name, wraps where it has to: it ran past
    // the row's edge and the rest of it could not be read.
    void aLongWordInACommentWraps() {
        const QString word = QStringLiteral("supercalifragilisticexpialidocious_filename_without_any_breaks_at_all.mov");
        const QString html = review::richBody(QStringLiteral("See %1 at 00:00:01:00, and <this>").arg(word), 24.0);
        QTextDocument document;
        document.setDefaultFont(tokens::font(tokens::reading));
        // As a QLabel with word wrap lays its text out.
        QTextOption wrapping = document.defaultTextOption();
        wrapping.setWrapMode(QTextOption::WordWrap);
        document.setDefaultTextOption(wrapping);
        document.setHtml(html);
        document.setTextWidth(330);
        QVERIFY2(document.idealWidth() <= 330.5, qPrintable(QString::number(document.idealWidth())));
        // It still says what it said.
        QString said = document.toPlainText();
        said.remove(QChar(0x200B));
        QVERIFY(said.contains(word));
        QVERIFY(said.contains(QStringLiteral("<this>")));
        QVERIFY(said.contains(QStringLiteral("00:00:01:00")));
        // A sentence of ordinary words is as it was.
        const QString plain = review::richBody(QStringLiteral("The bevel on the title reads well"), 24.0);
        QVERIFY(!plain.contains(QChar(0x200B)));
    }

    // The accessibility press on "N changes" toggles it without a click: the card has to follow, as
    // it does a click, or the chevron turns and nothing opens.
    void aProposalCardFollowsItsExpanderHoweverItIsToggled() {
        SeededWindow seeded;
        ProposalCard *sea = seeded.window.m_chat->card("proposal-kg");
        QVERIFY(sea);
        sea->setOpen(false);
        QSignalSpy toggled(sea, &ProposalCard::openToggled);
        sea->changesLink()->toggle();
        QVERIFY(sea->changesLink()->isChecked());
        QVERIFY(!sea->m_changes->isHidden());
        QCOMPARE(toggled.size(), 1);
        sea->changesLink()->toggle();
        QVERIFY(sea->m_changes->isHidden());
        QCOMPARE(toggled.size(), 2);
        // Set from outside it says nothing, as ever.
        sea->setOpen(true);
        QCOMPARE(toggled.size(), 2);
        // And a click is one toggle.
        sea->changesLink()->click();
        QVERIFY(sea->m_changes->isHidden());
        QCOMPARE(toggled.size(), 3);
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::CommentsTest)
#include "comments_test.moc"
