// MainWindow's comments panel (the agent chat), proposals, Ask and next steps.
// Moved verbatim from main_window.cpp (Q-SKELETON, W1); Q-COMMENTS owns it in W2.

#include "main_window.h"
#include "ask_prompt.h"
#include "bridge.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "inspector_panel.h"
#include "effects_panel.h"
#include "presets_panel.h"
#include "project_launcher.h"
#include "project_panel.h"
#include "source_monitor_panel.h"
#include "workspace.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "media_artwork.h"
#include "timeline_widget.h"
#include "toolbar_widgets.h"
#include "player_overlay.h"
#include "program_level.h"
#include "viewer_widget.h"
#ifdef Q_OS_MACOS
#include "macos_window.h"
#endif

#include <QAbstractTextDocumentLayout>
#include <QActionGroup>
#include <QApplication>
#include <QButtonGroup>
#include <QClipboard>
#include <QColorDialog>
#include <QCursor>
#include <QDateTime>
#include <QDesktopServices>
#include <QDialog>
#include <QDialogButtonBox>
#include <QDir>
#include <QFile>
#include <QFileDialog>
#include <QFileInfo>
#include <QFontDatabase>
#include <QGridLayout>
#include <QHBoxLayout>
#include <QInputDialog>
#include <QIcon>
#include <QImage>
#include <QJsonDocument>
#include <QKeySequence>
#include <QLayout>
#include <QMenu>
#include <QMenuBar>
#include <QMessageBox>
#include <QMouseEvent>
#include <QPaintEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPixmap>
#include <QPolygonF>
#include <QPlainTextEdit>
#include <QPushButton>
#include <QProcess>
#include <QRegularExpression>
#include <QSaveFile>
#include <QScrollBar>
#include <QSettings>
#include <QScreen>
#include <QShortcut>
#include <QShowEvent>
#include <QSignalBlocker>
#include <QStatusBar>
#include <QTextBlock>
#include <QTextDocument>
#include <QTextFrame>
#include <QToolBar>
#include <QUrl>
#include <QVBoxLayout>
#include <QWindow>

#include <DockManager.h>
#include <DockWidget.h>
#include <DockWidgetTab.h>

#include <algorithm>
#include <numeric>
#include <limits>
#include <utility>
#include "wiring/main_window_shared.h"
#include "browser_panel.h"
#include "deliver_tray.h"
#include "editor_bus.h"
#include "finish_tray.h"
#include "inspector_column.h"
#include "program_strip.h"
#include "stage_rail.h"

namespace kerf {

using namespace window_parts;

namespace {

// A script line's words, a quoted phrase ("A lower third") one word.
QStringList scriptWords(const QString &line) {
    QStringList words;
    QString word;
    bool quoted = false;
    for (const QChar c : line) {
        if (c == QLatin1Char('"')) quoted = !quoted;
        if (c.isSpace() && !quoted) {
            if (!word.isEmpty()) words << std::exchange(word, QString());
            continue;
        }
        word += c;
    }
    if (!word.isEmpty()) words << word;
    return words;
}

// Seconds in a script's amount or position ("-36f", "+1.5s", "96f",
// "00:00:12:00"), or NaN when it is something else (an anchor).
double scriptSeconds(QString text, double fps) {
    text = text.trimmed();
    double sign = 1.0;
    if (text.startsWith(QLatin1Char('-')) || text.startsWith(QLatin1Char('+'))) {
        sign = text.startsWith(QLatin1Char('-')) ? -1.0 : 1.0;
        text = text.mid(1);
    }
    bool ok = false;
    double value = 0.0;
    if (text.endsWith(QLatin1Char('f'))) value = text.chopped(1).toDouble(&ok) / std::max(1.0, fps);
    else if (text.endsWith(QLatin1Char('s'))) value = text.chopped(1).toDouble(&ok);
    else {
        const QStringList parts = text.split(QRegularExpression(QStringLiteral("[:;]")));
        if (parts.size() == 4) {
            ok = true;
            double fields[4];
            for (int index = 0; index < 4; ++index) {
                bool part = false;
                fields[index] = parts.at(index).toDouble(&part);
                ok = ok && part;
            }
            value = fields[0] * 3600 + fields[1] * 60 + fields[2] + fields[3] / std::max(1.0, fps);
        }
    }
    return ok ? sign * value : std::numeric_limits<double>::quiet_NaN();
}

// "0:12", "1:04:30": a position as a reader says it.
QString clockTime(double seconds) {
    const qint64 whole = std::llround(std::max(0.0, seconds));
    const qint64 hours = whole / 3600;
    const QString rest = QStringLiteral("%1:%2").arg((whole / 60) % 60).arg(whole % 60, 2, 10, QLatin1Char('0'));
    return hours ? QStringLiteral("%1:%2").arg(hours).arg(rest.rightJustified(5, QLatin1Char('0'))) : rest;
}

// "−1.5 s", "+12 s": an amount, its sign always shown (a true minus).
QString signedSeconds(double seconds) {
    const QString magnitude = QString::number(std::abs(seconds), 'f', std::abs(seconds) < 10 ? 1 : 0);
    const QString trimmed = magnitude.endsWith(QStringLiteral(".0")) ? magnitude.chopped(2) : magnitude;
    return QStringLiteral("%1%2 s").arg(seconds < 0 ? QStringLiteral("−") : QStringLiteral("+"), trimmed);
}

// The value of `key=` among a line's words, unquoted.
QString scriptValue(const QStringList &words, const QString &key) {
    const QString prefix = key + QLatin1Char('=');
    for (const QString &word : words)
        if (word.startsWith(prefix)) {
            QString value = word.mid(prefix.size());
            if (value.size() >= 2 && value.startsWith(QLatin1Char('"')) && value.endsWith(QLatin1Char('"'))) value = value.mid(1, value.size() - 2);
            return value;
        }
    return {};
}

} // namespace

void MainWindow::askAboutInChat(const QString &subject) {
    // The conversation, open, with the subject in the composer.
    if (!chatOpen()) setChatOpen(true);
    QString text = m_chatInput->toPlainText().trimmed();
    if (!text.startsWith(subject)) text = subject + QLatin1Char(' ') + text;
    else text += QLatin1Char(' ');
    m_chatInput->setPlainText(text.endsWith(QLatin1Char(' ')) ? text : text + QLatin1Char(' '));
    QTextCursor cursor = m_chatInput->textCursor();
    cursor.movePosition(QTextCursor::End);
    m_chatInput->setTextCursor(cursor);
    m_chatInput->setFocus(Qt::OtherFocusReason);
}

void MainWindow::sendChat() {
    const QString text = m_chatInput->toPlainText().trimmed();
    if (text.isEmpty() || !sendAgent(text, QJsonObject{}, QString())) return;
    m_chatInput->clear();
}

bool MainWindow::sendAgent(const QString &text, const QJsonObject &focus, const QString &focusLabel) {
    QJsonObject request = agentSettingsPayload();
    request.insert(QStringLiteral("type"), QStringLiteral("agent"));
    request.insert(QStringLiteral("text"), text);
    if (!focus.isEmpty()) request.insert(QStringLiteral("focus"), focus);
    // The AI bar names what Kerf works on once the bridge reports it busy.
    m_agentFocusLabel = focusLabel;
    if (!sendSimple(request)) return false;
    m_lastFocus = focus;
    m_lastFocusLabel = focusLabel;
    showNextSteps(QJsonArray{});
    m_agentBusy = true;
    m_agentText.clear();
    m_agentStatus->setText(QStringLiteral("Working"));
    m_chatInput->setEnabled(false);
    m_abortButton->setEnabled(true);
    updateChatControls();
    if (!focusLabel.isEmpty()) appendConversationNote(QStringLiteral("Focus"), focusLabel);
    appendConversation(QStringLiteral("You"), text);
    appendConversationNote(QStringLiteral("Progress"), QStringLiteral("Request sent"));
    addActivity(QStringLiteral("agent"), QStringLiteral("Request sent"));
    updateAiBar();
    updateCommandStates();
    return true;
}

void MainWindow::askAi() {
    // Anchor to the selected clip, or to the timeline at the playhead.
    const QJsonObject clip = selectedClipObject();
    const QRect bounds = clip.isEmpty() ? QRect() : m_timeline->clipBounds(m_selectedClip);
    const QRect visible = m_timelineScroll->viewport()->rect();
    QPoint anchor = m_timelineScroll->viewport()->mapToGlobal(QPoint(visible.center().x(), 48));
    if (!bounds.isEmpty()) {
        const QPoint top = m_timeline->mapTo(m_timelineScroll->viewport(), bounds.topLeft() + QPoint(bounds.width() / 2, 0));
        if (visible.contains(QPoint(std::clamp(top.x(), 0, visible.width() - 1), std::clamp(top.y(), 0, visible.height() - 1))))
            anchor = m_timelineScroll->viewport()->mapToGlobal(QPoint(std::clamp(top.x(), 24, visible.width() - 24), std::max(top.y(), 24)));
    }
    if (clip.isEmpty()) {
        m_askPrompt->open(anchor, QStringLiteral("Sequence at %1").arg(timelineTimecode(m_playhead, m_fps)), false);
        return;
    }
    const double start = clip.value(QStringLiteral("timelineStart")).toDouble();
    const double end = start + clip.value(QStringLiteral("duration")).toDouble();
    m_askPrompt->open(anchor, QStringLiteral("%1  ·  %2–%3").arg(clipName(clip), timelineTimecode(start, m_fps), timelineTimecode(end, m_fps)), true);
}

void MainWindow::askSubmitted(const QString &text, const QString &scope) {
    const QJsonObject clip = selectedClipObject();
    QJsonObject focus{{QStringLiteral("scope"), scope}};
    QString label;
    if (!clip.isEmpty()) {
        const qint64 start = clipFrames(clip, QStringLiteral("startFrame"));
        focus.insert(QStringLiteral("clip"), m_selectedClip);
        focus.insert(QStringLiteral("fromFrame"), start);
        focus.insert(QStringLiteral("toFrame"), start + clipFrames(clip, QStringLiteral("durationFrames")));
        label = scope == QStringLiteral("clip") ? clipName(clip) : QStringLiteral("Sequence around %1").arg(clipName(clip));
    } else {
        focus.insert(QStringLiteral("fromFrame"), frameAt(m_playhead));
        focus.insert(QStringLiteral("toFrame"), frameAt(m_playhead));
        label = QStringLiteral("Sequence at %1").arg(timelineTimecode(m_playhead, m_fps));
    }
    sendAgent(text, focus, label);
}

void MainWindow::showNextSteps(const QJsonArray &steps) {
    m_steps = steps;
    for (QWidget *row : {m_nextSteps, m_chatNextSteps}) {
        while (QLayoutItem *item = row->layout()->takeAt(0)) {
            // A pressed button sends the request that clears it.
            if (item->widget()) item->widget()->deleteLater();
            delete item;
        }
        for (const QJsonValue &value : steps) {
            const QJsonObject step = value.toObject();
            const QString request = step.value(QStringLiteral("request")).toString();
            auto *button = new QPushButton(step.value(QStringLiteral("label")).toString());
            button->setObjectName(QStringLiteral("nextStep"));
            // A pill: its focus ring follows its round ends.
            button->setProperty("bezelRadius", 11);
            button->setToolTip(request);
            button->setCursor(Qt::PointingHandCursor);
            connect(button, &QPushButton::clicked, this, [this, request] {
                if (!m_agentBusy) sendAgent(request, m_lastFocus, m_lastFocusLabel);
            });
            row->layout()->addWidget(button);
        }
    }
    m_chatNextSteps->setVisible(!steps.isEmpty());
    updateAiBar();
}

void MainWindow::updateChatControls() {
    const bool hasText = !m_chatInput->toPlainText().trimmed().isEmpty();
    m_sendButton->setEnabled(m_connected && hasText && !m_agentBusy);
    m_abortButton->setEnabled(m_agentBusy);
    m_abortButton->setVisible(m_agentBusy);
    m_sendButton->setVisible(!m_agentBusy);
    // The composer's underline shows the focus of any field in it (the
    // message, the model), in place of a ring; a button in it rings itself.
    const QWidget *focus = QApplication::focusWidget();
    const bool focused = focus && m_chatComposer->isAncestorOf(focus) && !qobject_cast<const QAbstractButton *>(focus);
    const QString state = !m_connected ? QStringLiteral("offline")
                          : m_agentBusy ? QStringLiteral("sending")
                          : focused ? QStringLiteral("focused") : QStringLiteral("ready");
    if (m_chatComposer->property("state").toString() != state) {
        m_chatComposer->setProperty("state", state);
        m_chatComposer->style()->unpolish(m_chatComposer);
        m_chatComposer->style()->polish(m_chatComposer);
        for (QWidget *part : m_chatComposer->findChildren<QWidget *>()) part->update();
        m_chatComposer->update();
    }
    m_chatInput->setToolTip(!m_connected ? QStringLiteral("Connect to start editing")
                            : m_agentBusy ? QStringLiteral("Kerf is working…")
                                          : QStringLiteral("Return sends · Shift-Return starts a new line"));
    // With a clip selected the request goes with it (the bridge sends the
    // selection), so the composer says so.
    const QJsonObject selected = selectedClipObject();
    m_chatInput->setPlaceholderText(!m_connected ? QStringLiteral("Your next edit starts here…")
                                   : m_agentBusy ? QStringLiteral("Working on your edit…")
                                   : !selected.isEmpty() ? QStringLiteral("Ask Kerf about %1…").arg(clipName(selected))
                                                         : QStringLiteral("Describe your next edit…"));
    m_chat->updateComposer(m_agentBusy);
    updateChatStatus();
}

void MainWindow::acceptProposal() {
    execute(QStringLiteral("proposal.accept"));
}

void MainWindow::rejectProposal() {
    execute(QStringLiteral("proposal.reject"));
}

void MainWindow::updateProposal(const QJsonValue &value) {
    m_proposalId.clear();
    m_proposalBlocked = false;
    bool pending = value.isObject() && value.toObject().value(QStringLiteral("status")).toString() == QStringLiteral("pending");
    m_acceptButton->setEnabled(m_connected && pending);
    m_rejectButton->setEnabled(m_connected && pending);
    m_chat->setProposalShown(pending, pending ? value.toObject().value(QStringLiteral("proposalId")).toString() : QString());
    if (!pending) {
        m_chat->setProposalChanges({});
        m_chat->setProposalSummary(QStringLiteral("No pending changes"));
        m_proposalText.clear();
        m_proposalDiff->clear();
        updateAiBar();
        return;
    }
    const QJsonObject proposal = value.toObject();
    m_proposalId = proposal.value(QStringLiteral("proposalId")).toString();
    const bool stale = proposal.value(QStringLiteral("freshness")).toString() == QStringLiteral("stale");
    const QString diff = proposal.value(QStringLiteral("documentDiff")).toString();
    // What it changes, one row each, from its script; the summary counts
    // those rows ("Trim, remove, add a title · 3 changes"); the compact
    // bar keeps the verbs ("Trim, remove, add a title"), and only the count
    // where it is too narrow for them.
    QList<ProposalChange> changes;
    QStringList verbs;
    for (const QString &line : proposal.value(QStringLiteral("script")).toString().split(QLatin1Char('\n'))) {
        const QString text = line.trimmed();
        if (text.isEmpty() || text.startsWith(QLatin1Char('#'))) continue;
        const ProposalChange change = describeScriptLine(text);
        changes.append(change);
        const QString verb = change.description.section(QLatin1Char(' '), 0, 0).toLower();
        const QString phrase = change.glyph == ProposalChange::Glyph::Title && verb == QStringLiteral("add") ? QStringLiteral("add a title") : verb;
        if (!verbs.contains(phrase)) verbs << phrase;
    }
    const QString count = changes.size() == 1 ? QStringLiteral("1 change") : QStringLiteral("%1 changes").arg(changes.size());
    QString verbsLine = verbs.size() <= 3 ? verbs.join(QStringLiteral(", ")) : QString();
    if (!verbsLine.isEmpty()) verbsLine = verbsLine.left(1).toUpper() + verbsLine.mid(1);
    const QString sentence = verbsLine.isEmpty() ? QString() : verbsLine + QStringLiteral(" · ");
    const QString stalePart = stale ? QStringLiteral(" · stale") : QString();
    m_chat->setProposalSummary(sentence + count + stalePart, verbsLine.isEmpty() ? count + stalePart : verbsLine + stalePart,
                               count + stalePart);
    m_chat->setProposalChanges(changes);
    QString text;
    if (stale) text += QStringLiteral("This proposal is out of date. Ask Kerf to prepare it again before accepting.\n\n");
    text += proposal.value(QStringLiteral("script")).toString();
    text += QStringLiteral("\n") + diff;
    m_proposalText = text;
    showProposalText();
    m_acceptButton->setEnabled(m_connected && !stale);
    m_proposalBlocked = stale;
    updateAiBar();
}

ProposalChange MainWindow::describeScriptLine(const QString &line) const {
    // A script line as a change a reader follows: what it does, to which
    // clip (by its name), and how much or where. Anything not known here
    // reads as the line itself.
    const QStringList words = scriptWords(line);
    ProposalChange change;
    change.description = line.left(1).toUpper() + line.mid(1);
    if (words.isEmpty()) return change;
    const QString verb = words.at(0);
    // The names said, so the short description can drop their extensions.
    QStringList names;
    const auto clipName = [this, &names](const QString &id) {
        const QJsonObject clip = clipObject(id);
        const QString name = clip.value(QStringLiteral("name")).toString();
        names << (name.isEmpty() ? id : name);
        return names.constLast();
    };
    const auto clipSpan = [this](const QString &id) {
        const QJsonObject clip = clipObject(id);
        if (clip.isEmpty()) return QString();
        const double start = clip.value(QStringLiteral("timelineStart")).toDouble();
        return QStringLiteral("%1\u2009\u2013\u2009%2").arg(clockTime(start), clockTime(start + clip.value(QStringLiteral("duration")).toDouble()));
    };
    const auto at = [this, &words](const QString &key, int position = -1) {
        QString value = scriptValue(words, key);
        if (value.isEmpty() && position >= 0 && position < words.size()) value = words.at(position);
        const double seconds = scriptSeconds(value, m_fps);
        return std::isnan(seconds) ? value : clockTime(seconds);
    };
    const QString subject = words.value(1);
    if (verb == QStringLiteral("trim") && words.size() >= 3) {
        const bool end = words.at(2) == QStringLiteral("out");
        change.glyph = ProposalChange::Glyph::Trim;
        change.description = QStringLiteral("Trim %1").arg(clipName(subject));
        // Which edge, and by how much: "end −1.5 s", "start to 0:04".
        const QString edge = end ? QStringLiteral("end") : QStringLiteral("start");
        const double amount = scriptSeconds(words.value(3), m_fps);
        if (!std::isnan(amount)) change.detail = QStringLiteral("%1 %2").arg(edge, signedSeconds(amount));
        else if (!scriptValue(words, QStringLiteral("to")).isEmpty()) change.detail = QStringLiteral("%1 to %2").arg(edge, at(QStringLiteral("to")));
        else change.detail = edge;
    } else if (verb == QStringLiteral("remove") || verb == QStringLiteral("delete") || verb == QStringLiteral("ripple-delete") ||
               verb == QStringLiteral("extract")) {
        change.glyph = ProposalChange::Glyph::Remove;
        change.description = QStringLiteral("Remove %1").arg(clipName(subject));
        change.detail = clipSpan(subject);
    } else if (verb == QStringLiteral("add") && words.size() >= 2) {
        const QString kind = words.at(1);
        QString text = scriptValue(words, QStringLiteral("text"));
        // Without its words in the line, the name the proposal gives it.
        for (const QJsonValue &track : m_snapshot.value(QStringLiteral("proposalTimeline")).toObject().value(QStringLiteral("tracks")).toArray())
            for (const QJsonValue &clip : track.toObject().value(QStringLiteral("clips")).toArray())
                if (text.isEmpty() && clip.toObject().value(QStringLiteral("id")).toString() == words.value(2))
                    text = clip.toObject().value(QStringLiteral("name")).toString();
        change.glyph = kind == QStringLiteral("title") ? ProposalChange::Glyph::Title : ProposalChange::Glyph::Add;
        change.description = !text.isEmpty() ? QStringLiteral("Add %1 “%2”").arg(kind, text) : QStringLiteral("Add a %1").arg(kind);
        const int atWord = int(words.indexOf(QStringLiteral("at")));
        const QString where = atWord >= 0 ? at(QString(), atWord + 1) : at(QStringLiteral("at"));
        if (!where.isEmpty()) change.detail = QStringLiteral("at %1").arg(where);
    } else if (verb == QStringLiteral("insert") || verb == QStringLiteral("overwrite")) {
        change.glyph = ProposalChange::Glyph::Add;
        const QString media = scriptValue(words, QStringLiteral("media"));
        QString name = media;
        for (const QJsonValue &value : m_project.value(QStringLiteral("sources")).toArray())
            if (value.toObject().value(QStringLiteral("id")).toString() == media) name = sourceName(value.toObject());
        names << name;
        change.description = QStringLiteral("%1 %2").arg(verb == QStringLiteral("insert") ? QStringLiteral("Insert") : QStringLiteral("Overwrite with"), name);
        if (!scriptValue(words, QStringLiteral("at")).isEmpty()) change.detail = QStringLiteral("at %1").arg(at(QStringLiteral("at")));
    } else if ((verb == QStringLiteral("move") || verb == QStringLiteral("roll") || verb == QStringLiteral("slide") ||
                verb == QStringLiteral("slip")) && words.size() >= 2) {
        change.glyph = ProposalChange::Glyph::Move;
        change.description = QStringLiteral("%1 %2").arg(verb.left(1).toUpper() + verb.mid(1), clipName(subject));
        const double amount = scriptSeconds(words.value(2), m_fps);
        if (!std::isnan(amount)) change.detail = signedSeconds(amount);
        else if (!scriptValue(words, QStringLiteral("at")).isEmpty()) change.detail = QStringLiteral("to %1").arg(at(QStringLiteral("at")));
        else if (!scriptValue(words, QStringLiteral("to")).isEmpty()) change.detail = QStringLiteral("to %1").arg(at(QStringLiteral("to")));
    } else if (verb == QStringLiteral("split") && words.size() >= 2) {
        change.glyph = ProposalChange::Glyph::Split;
        change.description = QStringLiteral("Split %1").arg(clipName(subject));
        if (!scriptValue(words, QStringLiteral("at")).isEmpty()) change.detail = QStringLiteral("at %1").arg(at(QStringLiteral("at")));
    } else if (verb == QStringLiteral("set") && words.size() >= 3) {
        QStringList keys;
        for (const QString &word : words.mid(2)) keys << word.section(QLatin1Char('='), 0, 0);
        const QString gain = scriptValue(words, QStringLiteral("gain"));
        if (!gain.isEmpty() || keys.contains(QStringLiteral("volume"))) {
            change.glyph = ProposalChange::Glyph::Level;
            change.description = QStringLiteral("Change %1’s level").arg(clipName(subject));
            bool ok = false;
            const double db = gain.toDouble(&ok);
            if (ok) change.detail = QStringLiteral("%1%2 dB").arg(db < 0 ? QStringLiteral("\u2212") : QString(), QString::number(std::abs(db), 'g', 3));
        } else {
            change.glyph = ProposalChange::Glyph::Other;
            change.description = QStringLiteral("Change %1’s %2").arg(clipName(subject), keys.join(QStringLiteral(", ")));
        }
    } else if (verb == QStringLiteral("fx") && words.size() >= 3) {
        change.glyph = ProposalChange::Glyph::Effect;
        const QString effect = words.value(3).section(QLatin1Char('.'), -1);
        change.description = words.at(2) == QStringLiteral("add") ? QStringLiteral("Add %1 to %2").arg(effect, clipName(subject))
                           : words.at(2) == QStringLiteral("remove") ? QStringLiteral("Remove an effect from %1").arg(clipName(subject))
                                                                       : QStringLiteral("Change the effects on %1").arg(clipName(subject));
    } else if (verb == QStringLiteral("mask") && words.size() >= 3) {
        change.glyph = ProposalChange::Glyph::Effect;
        change.description = words.at(2) == QStringLiteral("add") ? QStringLiteral("Add a mask to %1").arg(clipName(subject))
                                                                   : QStringLiteral("Change the masks on %1").arg(clipName(subject));
    } else if (verb == QStringLiteral("transition")) {
        change.glyph = ProposalChange::Glyph::Add;
        const QString kind = scriptValue(words, QStringLiteral("fx"));
        change.description = QStringLiteral("Add a %1").arg(kind.isEmpty() ? QStringLiteral("transition") : kind.section(QLatin1Char('.'), -1));
        const double length = scriptSeconds(scriptValue(words, QStringLiteral("dur")), m_fps);
        if (!std::isnan(length)) change.detail = signedSeconds(length).mid(1);
    } else if (verb == QStringLiteral("captions") && words.size() >= 2) {
        change.glyph = ProposalChange::Glyph::Title;
        change.description = QStringLiteral("Caption %1").arg(clipName(subject));
    }
    // For a narrow row: "Trim fisher-close" (the tooltip keeps the full name).
    change.shortDescription = change.description;
    // Only a media file's own extension goes: names from media come without
    // one already (see sourceName), and "clip.v2" keeps its ".v2".
    static const QStringList mediaSuffixes{
        QStringLiteral("mov"), QStringLiteral("mp4"), QStringLiteral("m4v"), QStringLiteral("mkv"), QStringLiteral("webm"),
        QStringLiteral("avi"), QStringLiteral("mxf"), QStringLiteral("wav"), QStringLiteral("mp3"), QStringLiteral("m4a"),
        QStringLiteral("aac"), QStringLiteral("aif"), QStringLiteral("aiff"), QStringLiteral("flac"), QStringLiteral("png"),
        QStringLiteral("jpg"), QStringLiteral("jpeg"), QStringLiteral("gif"), QStringLiteral("tif"), QStringLiteral("tiff")};
    for (const QString &name : std::as_const(names)) {
        const QFileInfo file(name);
        const QString base = file.completeBaseName();
        if (base.isEmpty() || !mediaSuffixes.contains(file.suffix(), Qt::CaseInsensitive)) continue;
        change.shortDescription.replace(name, base);
    }
    return change;
}

void MainWindow::showProposalText() {
    // Added lines in the success green, removed ones in the error red, and
    // the script and diff headers in secondary text.
    const EditorTheme &theme = editorTheme();
    m_proposalDiff->clear();
    QTextCursor cursor(m_proposalDiff->document());
    const QStringList lines = m_proposalText.split(QLatin1Char('\n'));
    for (qsizetype index = 0; index < lines.size(); ++index) {
        const QString &line = lines.at(index);
        const bool header = line.startsWith(QStringLiteral("+++")) || line.startsWith(QStringLiteral("---"));
        QTextCharFormat format;
        format.setForeground(!header && line.startsWith(QLatin1Char('+')) ? theme.success
                             : !header && line.startsWith(QLatin1Char('-')) ? theme.error
                                                                             : theme.textSecondary);
        if (index > 0) cursor.insertBlock();
        cursor.insertText(line, format);
    }
    m_proposalDiff->moveCursor(QTextCursor::Start);
}

void MainWindow::appendConversation(const QString &speaker, const QString &text) {
    if (text.isEmpty()) return;
    const bool user = speaker == QStringLiteral("You");
    const bool error = speaker == QStringLiteral("Error");
    CommentRow *row = m_chat->addComment(user ? CommentRow::Kind::User : error ? CommentRow::Kind::Error : CommentRow::Kind::Kerf, text);
    if (user) {
        // A new turn: Kerf's steps from now on belong to its next reply.
        m_agentRow = nullptr;
        m_pendingReplies.clear();
    } else if (speaker == QStringLiteral("Kerf")) {
        m_agentRow = row;
        for (const QString &reply : std::as_const(m_pendingReplies)) row->addReply(reply);
        m_pendingReplies.clear();
    }
    updateChatCount();
}

void MainWindow::updateChatCount() {
    const qsizetype count = m_chat->rows().size();
    m_rail->chatButton()->setLabel(count ? QString::number(count) : QStringLiteral("Chat"));
}

void MainWindow::updateChatTips() {
    if (!m_commands) return;
    const QString close = m_commands->labelWithShortcut(QStringLiteral("window.assistant"), QStringLiteral("Close Kerf Assistant"));
    m_chat->closeButton()->setToolTip(close);
    RailButton *rail = m_rail->chatButton();
    rail->setToolTip(rail->isChecked() ? close
                                       : m_commands->labelWithShortcut(QStringLiteral("window.assistant"), QStringLiteral("Open Kerf Assistant")));
    rail->setAccessibleName(rail->isChecked() ? QStringLiteral("Close Kerf Assistant") : QStringLiteral("Open Kerf Assistant"));
}

bool MainWindow::chatOpen() const {
    // The viewer's panel, open, in an open Program monitor: with the
    // Program monitor closed the timeline's bar stands in for it and its
    // switches read as closed.
    const ads::CDockWidget *program = m_workspace ? m_workspace->panel(QStringLiteral("program")) : nullptr;
    return m_viewer && m_viewer->chatOpen() && program && !program->isClosed();
}

void MainWindow::setChatOpen(bool open) {
    m_viewer->setChatOpen(open);
    QSettings().setValue(QStringLiteral("viewer/chatOpen"), open);
    // The workspace's stage as it is now, for Save Changes.
    rememberStage();
    // It lives in the Program monitor's stage, so that comes forward with it.
    if (ads::CDockWidget *program = open && m_workspace ? m_workspace->panel(QStringLiteral("program")) : nullptr) {
        if (program->isClosed()) program->toggleView(true);
        program->setAsCurrentTab();
    }
    syncChatSwitches();
}

void MainWindow::syncChatSwitches() {
    // The toolbar's bubble, Window ▸ Kerf Assistant and the rail's button
    // read open only while the conversation is open and in sight.
    const bool open = chatOpen();
    if (m_commands) {
        for (const QString &id : {QStringLiteral("window.assistant"), QStringLiteral("view.comments")})
            if (QAction *chat = m_commands->action(id)) {
                const QSignalBlocker blocker(chat);
                chat->setChecked(open);
            }
    }
    {
        const QSignalBlocker blocker(m_rail->chatButton());
        m_rail->chatButton()->setChecked(m_viewer->chatOpen());
    }
    updateChatTips();
    updateAiBar();
}

void MainWindow::showActivity() {
    QWidget *more = m_chat->moreButton();
    m_activityPopup->adjustSize();
    m_activityPopup->move(more->mapToGlobal(QPoint(more->width() - m_activityPopup->width(), more->height() + 4)));
    m_activityPopup->show();
}

void MainWindow::updateChatStatus() {
    // A quiet line after the latest comment while there is something to
    // say: Kerf at work (its three dots and its latest step), connecting,
    // offline, stopped or failed. Ready and in review say nothing; the
    // pinned proposal speaks for review.
    const QString state = m_agentStatus->text();
    m_chat->statusRow()->setVisible(state != QStringLiteral("Ready") && state != QStringLiteral("Review"));
    m_chatActivity->setVisible(m_agentBusy && !m_chatActivity->text().isEmpty());
}

void MainWindow::appendConversationNote(const QString &kind, const QString &text) {
    if (text.isEmpty()) return;
    if (kind == QStringLiteral("Agent error")) {
        appendConversation(QStringLiteral("Error"), text);
        return;
    }
    const QString detail = text.simplified();
    // A tool's work is a reply under Kerf's comment for this turn, folded
    // behind "▸ 2 steps"; until Kerf writes, it waits for that comment.
    if (kind == QStringLiteral("Tool")) {
        if (m_agentRow) m_agentRow->addReply(detail);
        else m_pendingReplies.append(detail);
    }
    m_chatActivity->setText(detail);
    m_chatActivity->setToolTip(QStringLiteral("%1 · %2").arg(kind, detail));
    updateChatStatus();
}

bool MainWindow::handleAgentEvent(const QJsonObject &event) {
    const QString type = event.value(QStringLiteral("type")).toString();
    if (type == QStringLiteral("agent")) {
        const QString kind = event.value(QStringLiteral("kind")).toString(QStringLiteral("agent"));
        const QString text = event.value(QStringLiteral("text")).toString();
        if (kind == QStringLiteral("delta")) {
            if (m_agentText.isEmpty() || !m_agentRow) appendConversation(QStringLiteral("Kerf"), text);
            else m_agentRow->appendText(text);
            m_agentText += text;
        } else if (kind == QStringLiteral("done")) {
            if (m_agentText.isEmpty()) appendConversation(QStringLiteral("Kerf"), text);
            m_agentText.clear();
            appendConversationNote(QStringLiteral("Progress"), QStringLiteral("Response complete"));
        } else if (kind == QStringLiteral("tool")) {
            appendConversationNote(QStringLiteral("Tool"), text);
            addActivity(QStringLiteral("tool"), text.left(120));
        } else if (kind == QStringLiteral("status")) {
            appendConversationNote(QStringLiteral("Progress"), text);
            addActivity(QStringLiteral("status"), text.left(120));
        } else {
            appendConversationNote(QStringLiteral("Agent"), text);
            addActivity(kind, text.left(120));
        }
    } else if (type == QStringLiteral("agent_busy")) {
        const bool busy = event.value(QStringLiteral("busy")).toBool();
        m_agentBusy = busy;
        if (!m_connected) m_agentStatus->setText(QStringLiteral("Offline"));
        else if (busy) m_agentStatus->setText(QStringLiteral("Thinking"));
        else if (m_agentRunState == QStringLiteral("failed")) m_agentStatus->setText(QStringLiteral("Failed"));
        else if (m_agentRunState == QStringLiteral("aborted")) m_agentStatus->setText(QStringLiteral("Stopped"));
        else if (m_agentRunState == QStringLiteral("awaiting_review")) m_agentStatus->setText(QStringLiteral("Review"));
        else m_agentStatus->setText(QStringLiteral("Ready"));
        m_chatInput->setEnabled(m_connected && !busy);
        updateChatControls();
        updateAiBar();
        updateCommandStates();
    } else if (type == QStringLiteral("agent_state")) {
        const QString state = event.value(QStringLiteral("state")).toString();
        m_agentRunState = state;
        const QString label = state == QStringLiteral("running") ? QStringLiteral("Thinking")
                              : state == QStringLiteral("awaiting_review") ? QStringLiteral("Review")
                              : state == QStringLiteral("failed") ? QStringLiteral("Failed")
                              : state == QStringLiteral("aborted") ? QStringLiteral("Stopped")
                              : QStringLiteral("Ready");
        m_agentStatus->setText(m_connected ? label : QStringLiteral("Offline"));
        updateChatStatus();
        if (state == QStringLiteral("failed")) {
            appendConversationNote(QStringLiteral("Agent error"), event.value(QStringLiteral("message")).toString());
        }
    } else if (type == QStringLiteral("agent_next")) {
        showNextSteps(event.value(QStringLiteral("steps")).toArray());
    } else {
        return false;
    }
    return true;
}

// The comments panel (Kerf's conversation) and the rail beside the picture.
void MainWindow::buildReview() {
    // Kerf's conversation lives in the Program monitor's stage beside the
    // picture, as a Short's comments open beside it: the rail's chat button
    // between them, and the panel as tall as the picture (see
    // ViewerWidget::stageLayout).
    m_chat = new ChatPanel;
    m_rail = new StageRail;
    m_chatScroll = m_chat->scroll();
    m_chatActivity = m_chat->activityLabel();
    m_agentStatus = m_chat->statusLabel();
    m_chatComposer = m_chat->composer();
    m_chatInput = m_chat->input();
    m_modelSelect = m_chat->modelSelect();
    m_sendButton = m_chat->sendButton();
    m_abortButton = m_chat->stopButton();
    m_proposalBox = m_chat->proposal();
    m_proposalSummary = m_chat->proposalSummary();
    m_proposalDiff = m_chat->proposalDiff();
    m_acceptButton = m_chat->acceptButton();
    m_rejectButton = m_chat->rejectButton();
    m_chatNextSteps = m_chat->chips();
    m_modelSelect->setToolTip(QStringLiteral("Model ID sent with each chat request. Suggestions follow the active connection in Settings."));
    m_chatInput->installEventFilter(this);
    connect(m_chatInput, &QTextEdit::textChanged, this, &MainWindow::updateChatControls);
    connect(m_sendButton, &QPushButton::clicked, this, &MainWindow::sendChat);
    m_abortButton->setEnabled(false);
    connect(m_abortButton, &QPushButton::clicked, this, [this] {
        if (!execute(QStringLiteral("ai.stop"))) return;
        m_abortButton->setEnabled(false);
        m_agentStatus->setText(QStringLiteral("Stopping"));
        appendConversationNote(QStringLiteral("Progress"), QStringLiteral("Stopping request…"));
    });
    // The composer follows the focus into and out of it (see updateChatControls).
    connect(qApp, &QApplication::focusChanged, this, [this](QWidget *from, QWidget *to) {
        if ((from && m_chatComposer->isAncestorOf(from)) || (to && m_chatComposer->isAncestorOf(to))) updateChatControls();
    });
    connect(m_chat, &ChatPanel::askAgain, this, [this](const QString &text) {
        if (!m_agentBusy && m_connected) sendAgent(text, QJsonObject{}, QString());
    });
    m_chat->moreButton()->setToolTip(QStringLiteral("Agent settings and activity"));
    connect(m_chat->moreButton(), &QAbstractButton::clicked, this, [this] {
        QMenu menu(this);
        menu.addAction(QStringLiteral("Agent Settings…"), this, [this] { triggerCommand(QStringLiteral("app.preferences")); });
        menu.addAction(QStringLiteral("Activity"), this, [this] { showActivity(); });
        QWidget *more = m_chat->moreButton();
        menu.exec(more->mapToGlobal(QPoint(more->width() - menu.sizeHint().width(), more->height() + 4)));
    });
    connect(m_chat->closeButton(), &QAbstractButton::clicked, this, [this] { setChatOpen(false); });
    // What the agent and the project did, one line each, from the ⋯ menu.
    m_activityPopup = new QFrame(this, Qt::Popup);
    m_activityPopup->setObjectName(QStringLiteral("activityPopup"));
    {
        auto *layout = new QVBoxLayout(m_activityPopup);
        layout->setContentsMargins(12, 10, 12, 10);
        layout->setSpacing(6);
        auto *title = new QLabel(QStringLiteral("Activity"));
        title->setObjectName(QStringLiteral("eyebrow"));
        layout->addWidget(title);
        m_activityList = new QListWidget;
        m_activityList->setObjectName(QStringLiteral("activityList"));
        m_activityList->setMinimumSize(320, 160);
        layout->addWidget(m_activityList);
    }
    connect(m_acceptButton, &QPushButton::clicked, this, &MainWindow::acceptProposal);
    connect(m_rejectButton, &QPushButton::clicked, this, &MainWindow::rejectProposal);
}

} // namespace kerf
