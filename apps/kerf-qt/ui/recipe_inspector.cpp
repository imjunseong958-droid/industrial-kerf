// What the node chosen in a shot's graph holds, said and set (recipe_inspector.h).

#include "recipe_inspector.h"
#include "components.h"
#include "icons.h"
#include "point_picture.h"
#include "settings_kit.h"
#include "signal_curve.h"

#include <QAction>
#include <QDir>
#include <QFileInfo>
#include <QHBoxLayout>
#include <QImageReader>
#include <QJsonObject>
#include <QKeyEvent>
#include <QLabel>
#include <QLineEdit>
#include <QMenu>
#include <QPainter>
#include <QPainterPath>
#include <QPlainTextEdit>
#include <QRandomGenerator>
#include <QScrollBar>
#include <QSignalBlocker>
#include <QVBoxLayout>

#include <algorithm>

namespace kerf {

using namespace tokens;
using graph::Edge;
using graph::Gives;
using graph::Kind;
using graph::Node;

namespace {

// What a field last showed of the recipe, kept on the field itself.
constexpr char kShown[] = "graphShown";
// The card's own padding, and how wide a row's name is: a recipe's are short
// words, a ComfyUI node's inputs longer ones (`filename_prefix`).
constexpr int kPad = 14;
constexpr int kLabelWidth = 78;
constexpr int kInputLabelWidth = 118;

QLabel *text(const QString &said, const TypeStyle &style, QRgb ink, bool wrap = false) {
    QLabel *label = wrap ? new ui::WrapLabel : new QLabel;
    label->setText(said);
    label->setFont(tokens::font(style));
    QPalette palette = label->palette();
    palette.setColor(QPalette::WindowText, color(ink));
    label->setPalette(palette);
    label->setWordWrap(wrap);
    label->setTextFormat(Qt::PlainText);
    if (wrap) label->setTextInteractionFlags(Qt::TextSelectableByMouse);
    return label;
}

// A prompt: one line of the recipe, however long, so it wraps as it is typed
// and Return leaves it as a line edit's does. It says it is done when it is
// left, or at Return.
class PromptEdit final : public QPlainTextEdit {
public:
    using QPlainTextEdit::QPlainTextEdit;
    std::function<void()> done;

protected:
    void keyPressEvent(QKeyEvent *event) override {
        if (event->key() == Qt::Key_Return || event->key() == Qt::Key_Enter) {
            if (done) done();
            return;
        }
        QPlainTextEdit::keyPressEvent(event);
    }
    void focusOutEvent(QFocusEvent *event) override {
        QPlainTextEdit::focusOutEvent(event);
        if (done) done();
    }
    void insertFromMimeData(const QMimeData *source) override {
        // What is pasted is one line, as the recipe's line is.
        QPlainTextEdit::insertFromMimeData(source);
        if (toPlainText().contains(QLatin1Char('\n'))) {
            const QString one = toPlainText().simplified();
            setPlainText(one);
            moveCursor(QTextCursor::End);
        }
    }
};

// One of the shot's takes: its picture, its number, what there is to say of
// it, and a tick on the one the shot is. A press has the shot be it.
class TakeRow final : public ui::KitButton {
public:
    // The picture's box in a row that has one.
    static constexpr QSize kPicture{57, 32};

    TakeRow(int number, const QString &said, bool chosen, const QImage &picture, QWidget *parent = nullptr)
        : KitButton(parent), m_number(number), m_said(said), m_chosen(chosen), m_picture(picture) {
        setObjectName(QStringLiteral("graphTake%1").arg(number));
        setAccessibleName(QStringLiteral("Take %1").arg(number));
        setAccessibleDescription(said);
        setFixedHeight(picture.isNull() ? height::h28 : kPicture.height() + 8);
        setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
        setCheckable(true);
        setChecked(chosen);
    }
    QString said() const { return m_said; }
    bool hasPicture() const { return !m_picture.isNull(); }

protected:
    // Which take the shot is is the document's to say, not a press's.
    void nextCheckState() override {}
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing, true);
        p.setRenderHint(QPainter::SmoothPixmapTransform, true);
        const QRectF row = QRectF(rect());
        if (isEnabled() && hoverProgress() > 0) {
            p.setPen(Qt::NoPen);
            p.setBrush(ui::mix(Qt::transparent, color(popupHover), hoverProgress()));
            p.drawRoundedRect(row, radius::card, radius::card);
        }
        if (focusVisible()) ui::paintFocusRing(p, row.adjusted(2, 2, -2, -2), radius::control);
        qreal left = row.left() + 8;
        if (!m_picture.isNull()) {
            // The take's picture, filling its box, its corners rounded as a card's.
            const QRectF box(row.left() + 4, row.center().y() - kPicture.height() / 2.0, kPicture.width(), kPicture.height());
            QPainterPath clip;
            clip.addRoundedRect(box, radius::control, radius::control);
            p.save();
            p.setClipPath(clip);
            const QSizeF fill = QSizeF(m_picture.size()).scaled(box.size(), Qt::KeepAspectRatioByExpanding);
            p.drawImage(QRectF(box.center().x() - fill.width() / 2, box.center().y() - fill.height() / 2, fill.width(), fill.height()), m_picture);
            p.restore();
            left = box.right() + 8;
        }
        p.setFont(tokens::font(m_chosen ? bodyName : body));
        p.setPen(color(isEnabled() || m_chosen ? ink : dis));
        const QString name = QStringLiteral("Take %1").arg(m_number);
        p.drawText(QRectF(left, row.top(), row.right() - 8 - left, row.height()), Qt::AlignLeft | Qt::AlignVCenter, name);
        const qreal nameWidth = QFontMetricsF(p.font()).horizontalAdvance(name);
        const qreal end = m_chosen ? row.right() - 8 - 14 - 6 : row.right() - 8;
        const QFont quiet = tokens::font(control);
        p.setFont(quiet);
        p.setPen(color(ink3));
        const qreal room = end - (left + nameWidth + 12);
        if (room > 24)
            p.drawText(QRectF(end - room, row.top(), room, row.height()), Qt::AlignRight | Qt::AlignVCenter,
                       QFontMetricsF(quiet).elidedText(m_said, Qt::ElideRight, room));
        if (m_chosen) icons::paint(p, QStringLiteral("g-check"), QRectF(row.right() - 8 - 14, row.center().y() - 7, 14, 14), color(accentInk), 14);
    }

private:
    int m_number;
    QString m_said;
    bool m_chosen;
    QImage m_picture;
};


// A value as a field shows it: a number without a needless fraction.
QString shownValue(const QJsonValue &value) {
    if (value.isString()) return value.toString();
    if (value.isBool()) return value.toBool() ? QStringLiteral("true") : QStringLiteral("false");
    if (value.isDouble()) return QString::number(value.toDouble(), 'g', 12);
    return {};
}

} // namespace

RecipeInspector::RecipeInspector(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("graphInspector"));
    setAccessibleName(QStringLiteral("Node"));
    setAttribute(Qt::WA_NoSystemBackground);
    // Its fields are wells in the card, as a menu's rows are steps on it.
    setStyleSheet(QStringLiteral(
                      "QLineEdit, QPlainTextEdit { background: %1; border: 1px solid %2; border-radius: 6px; color: %3; }"
                      "QLineEdit { padding: 0px 8px; min-height: 22px; max-height: 22px; }"
                      "QPlainTextEdit { padding: 4px 5px; }"
                      "QLineEdit:hover, QPlainTextEdit:hover { border-color: %4; }"
                      "QLineEdit:focus, QPlainTextEdit:focus { background: %1; border-color: %5; }"
                      "QLineEdit:disabled { color: %6; }")
                      .arg(color(well).name(), color(line2).name(QColor::HexArgb), color(ink).name(),
                           color(lineh).name(QColor::HexArgb), color(accent).name(), color(dis).name()));
    auto *outer = new QVBoxLayout(this);
    outer->setContentsMargins(1, 1, 1, 1);
    m_scroll = new ui::FadeScrollArea;
    m_scroll->setObjectName(QStringLiteral("graphInspectorScroll"));
    m_scroll->setFadeColor(color(popup));
    m_scroll->setWidgetResizable(true);
    m_scroll->setFrameShape(QFrame::NoFrame);
    m_scroll->setStyleSheet(QStringLiteral("QScrollArea, QScrollArea > QWidget > QWidget { background: transparent; }"));
    m_scroll->viewport()->setAutoFillBackground(false);
    outer->addWidget(m_scroll);
}

void RecipeInspector::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const QRectF card = QRectF(rect());
    p.setPen(Qt::NoPen);
    p.setBrush(color(popup));
    p.drawRoundedRect(card, radius::panel, radius::panel);
    ui::paintInsetLine(p, card, radius::panel, color(line2));
}

int RecipeInspector::heightFor(int most) const {
    if (!m_content) return 0;
    m_content->layout()->activate();
    const int width = std::max(0, this->width() - 2);
    const int wanted = (m_content->layout()->hasHeightForWidth() ? m_content->layout()->totalHeightForWidth(width)
                                                                 : m_content->layout()->totalSizeHint().height()) + 2;
    return std::min(wanted, most);
}

QString RecipeInspector::shape(const graph::Recipe &recipe, const Node &node) const {
    QStringList parts{node.id, QString::number(int(node.kind)), QString::number(node.line), QString::number(graph::isMissing(node))};
    for (const QJsonValue value : node.holds.value(QStringLiteral("knobs")).toArray()) parts << value.toObject().value(QStringLiteral("name")).toString();
    for (const Edge &edge : recipe.into(node.id)) parts << QStringLiteral("%1[%2]<%3").arg(edge.port).arg(edge.nth).arg(edge.from);
    // A workflow's node: which inputs it has and what each is, and of a blank what the line gives.
    for (const QJsonValue value : node.holds.value(QStringLiteral("inputs")).toArray()) {
        const QJsonObject input = value.toObject();
        parts << QStringLiteral("%1:%2").arg(input.value(QStringLiteral("name")).toString()).arg(int(graph::inputKind(input)));
        if (graph::inputKind(input) != graph::InputKind::Value) parts << graph::valueText(input.value(QStringLiteral("value")));
        // What the server takes there, once asked: a choice is a menu and shows what is chosen.
        if (const QJsonArray choices = input.value(QStringLiteral("choices")).toArray(); !choices.isEmpty())
            parts << QString::number(choices.size()) << graph::valueText(input.value(QStringLiteral("value")));
        parts << graph::valueText(input.value(QStringLiteral("min"))) << graph::valueText(input.value(QStringLiteral("max")));
        for (const QJsonValue blank : input.value(QStringLiteral("blanks")).toArray()) parts << blankSays(blank.toString());
    }
    parts << node.holds.value(QStringLiteral("runner")).toObject().value(QStringLiteral("workflow")).toString()
          << node.holds.value(QStringLiteral("workflow")).toString();
    for (const QJsonValue value : node.holds.value(QStringLiteral("takes")).toArray()) {
        const QJsonObject take = value.toObject();
        parts << QStringLiteral("%1%2%3").arg(take.value(QStringLiteral("number")).toInt()).arg(take.value(QStringLiteral("chosen")).toBool() ? '*' : ' ')
                     .arg(take.value(QStringLiteral("changed")).toArray().size());
        parts << QString::number(take.value(QStringLiteral("kept")).toBool());
    }
    // What a take reads beside its guide, and the pictures there are to show.
    if (node.kind == Kind::Takes) {
        parts << m_readState << QString::number(m_best);
        for (const QJsonValue value : node.holds.value(QStringLiteral("takes")).toArray()) {
            const QJsonObject take = value.toObject();
            parts << QString::number(m_readings.value(take.value(QStringLiteral("number")).toInt()).value(QStringLiteral("overlap")).toDouble(-1))
                  << QString::number(m_pictures.contains(take.value(QStringLiteral("path")).toString()));
        }
    }
    if (const QJsonObject made = node.holds.value(QStringLiteral("made")).toObject(); !made.isEmpty())
        parts << made.value(QStringLiteral("path")).toString() << QString::number(m_pictures.contains(made.value(QStringLiteral("path")).toString()));
    parts << QString::number(node.holds.value(QStringLiteral("from")).isNull());
    // How far reading its clip from footage has got.
    if (node.kind == Kind::Moves || node.kind == Kind::Render)
        parts << node.holds.value(QStringLiteral("matte")).toString() << QString::number(node.holds.value(QStringLiteral("there")).toBool())
              << m_pointing.clip << m_pointing.of << m_pointing.state << m_pointing.says << QString::number(m_pointing.picture.cacheKey());
    for (const QJsonValue sound : node.holds.value(QStringLiteral("sounds")).toArray()) parts << sound.toObject().value(QStringLiteral("clip")).toString();
    // What is said as text, not set: another word is another card.
    if (node.kind != Kind::Model && node.kind != Kind::Render && node.kind != Kind::Signal) parts << node.title << node.detail;
    parts << node.holds.value(QStringLiteral("wants")).toString() << node.holds.value(QStringLiteral("about")).toString()
          << node.holds.value(QStringLiteral("pass")).toString() << QString::number(m_making);
    return parts.join(QLatin1Char('\x1f'));
}

QHash<QString, QString> RecipeInspector::values(const Node &node) const {
    QHash<QString, QString> shown;
    const QJsonObject &holds = node.holds;
    if (node.kind == Kind::Model) {
        shown.insert(QStringLiteral("seed"), holds.value(QStringLiteral("seed")).isNull()
                                                 ? QString() : QString::number(holds.value(QStringLiteral("seed")).toVariant().toULongLong()));
        shown.insert(QStringLiteral("seconds"), shownValue(holds.value(QStringLiteral("seconds"))));
        shown.insert(QStringLiteral("from"), shownValue(holds.value(QStringLiteral("from"))));
        shown.insert(QStringLiteral("size"), holds.value(QStringLiteral("size")).toString());
        for (const QJsonValue value : holds.value(QStringLiteral("knobs")).toArray()) {
            const QJsonObject knob = value.toObject();
            shown.insert(QStringLiteral("knob.") + knob.value(QStringLiteral("name")).toString(), knob.value(QStringLiteral("value")).toString());
        }
    } else if (node.kind == Kind::Part) {
        for (const QJsonValue value : holds.value(QStringLiteral("inputs")).toArray()) {
            const QJsonObject input = value.toObject();
            if (graph::inputKind(input) == graph::InputKind::Value)
                shown.insert(QStringLiteral("input.") + input.value(QStringLiteral("name")).toString(), graph::valueText(input.value(QStringLiteral("value"))));
        }
    } else if (node.kind == Kind::Render) {
        shown.insert(QStringLiteral("sequence"), holds.value(QStringLiteral("sequence")).toString());
        shown.insert(QStringLiteral("size"), holds.value(QStringLiteral("size")).toString());
        shown.insert(QStringLiteral("rate"), holds.value(QStringLiteral("rate")).toString());
        shown.insert(QStringLiteral("matte"), holds.value(QStringLiteral("matte")).toString());
        shown.insert(QStringLiteral("from"), holds.value(QStringLiteral("from")).toString());
        shown.insert(QStringLiteral("seconds"), holds.value(QStringLiteral("seconds")).toString());
    } else if (node.kind == Kind::Join) {
        const int lap = holds.value(QStringLiteral("lap")).toInt();
        shown.insert(QStringLiteral("lap"), lap > 0 ? QString::number(lap) : QString());
    }
    return shown;
}

void RecipeInspector::setNode(const graph::Recipe &recipe, const QString &id) {
    const Node *node = recipe.node(id);
    if (!node && recipe.parts) {
        // No node of a workflow chosen: the workflow itself, and what fills its blanks.
        QStringList said{QStringLiteral("workflow"), m_workflow.name, m_workflow.origin, m_workflow.file, m_workflow.about};
        for (auto blank = m_workflow.blanks.constBegin(); blank != m_workflow.blanks.constEnd(); ++blank) said << blank.key() << blankSays(blank.key());
        const QString now = said.join(QLatin1Char('\x1f'));
        m_id.clear();
        m_line = 0;
        if (now == m_shape && m_content) return;
        m_shape = now;
        rebuildWorkflow();
        emit contentChanged();
        return;
    }
    if (!node) {
        // No node chosen: the shot itself, and what is worth knowing before it is made.
        const QString now = QStringList{QStringLiteral("shot"), QString::number(recipe.said), recipe.media, recipe.name, recipe.recipe, recipe.error, recipe.cannot,
                                        recipe.modelsError, recipe.notes.join(QLatin1Char('\n'))}.join(QLatin1Char('\x1f'));
        m_id.clear();
        m_line = 0;
        if (now == m_shape && m_content) return;
        m_shape = now;
        rebuildShot(recipe);
        emit contentChanged();
        return;
    }
    const QString now = shape(recipe, *node);
    m_id = id;
    m_line = node->line;
    if (now == m_shape && m_content) {
        refresh(*node);
        return;
    }
    m_shape = now;
    rebuild(recipe, *node);
    emit contentChanged();
}

void RecipeInspector::startContent() {
    m_fields.clear();
    m_switches.clear();
    m_prompt = nullptr;
    m_curve = nullptr;
    m_takeRows.clear();
    if (QWidget *old = m_scroll->takeWidget()) old->deleteLater();
    m_content = new QWidget;
    m_content->setObjectName(QStringLiteral("graphInspectorContent"));
    m_column = new QVBoxLayout(m_content);
    m_column->setContentsMargins(kPad, kPad, kPad, kPad);
    m_column->setSpacing(space::s8);
}

void RecipeInspector::rebuildShot(const graph::Recipe &recipe) {
    startContent();
    if (recipe.isValid()) {
        QLabel *title = text(recipe.name, tokens::title, ink);
        title->setObjectName(QStringLiteral("graphInspectorTitle"));
        m_column->addWidget(title);
        addNote(recipe.recipe, ink3)->setFont(tokens::font(monoSmall));
        if (!recipe.error.isEmpty())
            addNote(QStringLiteral("This recipe does not read: %1").arg(recipe.error), status::failed)->setObjectName(QStringLiteral("graphError"));
        if (!recipe.cannot.isEmpty())
            addNote(QStringLiteral("It cannot be made as things stand: %1").arg(recipe.cannot), status::failed)->setObjectName(QStringLiteral("graphCannot"));
        if (!recipe.modelsError.isEmpty()) addNote(recipe.modelsError, status::failed);
        if (!recipe.notes.isEmpty()) {
            // What a making would say first, each cheaper to mend before one than after.
            addSection(QStringLiteral("Before it is made"));
            for (const QString &note : recipe.notes) addNote(note, status::request)->setProperty("graphNote", true);
        }
        if (recipe.error.isEmpty() && recipe.said)
            addNote(QStringLiteral("Choose a node to see and set what it holds. A change is written into the recipe, which keeps its "
                                   "comments and its order; Make then writes a new take."));
    }
    m_column->addStretch(1);
    m_scroll->setWidget(m_content);
    m_content->show();
}

void RecipeInspector::setWorkflow(const graph::Workflow &workflow) { m_workflow = workflow; }

QString RecipeInspector::blankSays(const QString &name) const {
    const QJsonObject blank = m_workflow.blanks.value(name).toObject();
    QString says = blank.value(QStringLiteral("says")).toString();
    if (says.isEmpty()) return {};
    says[0] = says.at(0).toUpper();
    const QJsonValue given = blank.value(QStringLiteral("given"));
    const QString by = blank.value(QStringLiteral("by")).toString();
    // What the line gives for it now; a file or a signal it gives none of leaves the node out.
    if (given.isString()) return QStringLiteral("%1. Now: %2").arg(says, given.toString());
    if (by == QLatin1String("file") || by == QLatin1String("signal") || by == QLatin1String("moves"))
        return QStringLiteral("%1. The line gives none, so the node is left out").arg(says);
    return says;
}

void RecipeInspector::setMaking(bool making) {
    if (making == m_making) return;
    m_making = making;
    for (QWidget *row : std::as_const(m_takeRows)) row->setEnabled(!making && !static_cast<QAbstractButton *>(row)->isChecked());
}

void RecipeInspector::setTakeReadings(const QHash<int, QJsonObject> &readings, int best, const QString &state) {
    m_readings = readings;
    m_best = best;
    m_readState = state;
}

void RecipeInspector::setPictures(const QHash<QString, QImage> &pictures) { m_pictures = pictures; }

void RecipeInspector::setShotPicture(const QString &clip, const QString &of, const QImage &picture) {
    // The one asked for last, and still waited for.
    if (m_pointing.clip != clip || m_pointing.of != of || m_pointing.state != QLatin1String("drawing")) return;
    m_pointing.picture = picture;
    m_pointing.state = QStringLiteral("pointing");
}

void RecipeInspector::setShotFollowed(const QString &clip, const QString &of, const QString &says, bool failed) {
    if (m_pointing.clip != clip || m_pointing.of != of) return;
    // A picture that could not be drawn leaves nothing to point on; a thing that could not
    // be followed leaves the picture, to point again.
    m_pointing.state = failed && !m_pointing.picture.isNull() ? QStringLiteral("pointing") : QStringLiteral("said");
    m_pointing.says = says;
    m_pointing.failed = failed;
    m_pointing.on.clear();
    m_pointing.off.clear();
}

void RecipeInspector::refresh(const Node &node) {
    const QHash<QString, QString> now = values(node);
    for (auto field = m_fields.constBegin(); field != m_fields.constEnd(); ++field) {
        const QString value = now.value(field.key());
        field.value()->setProperty(kShown, value);
        // What is being typed is the person's until they leave it.
        if (field.value()->hasFocus() && field.value()->isModified()) continue;
        if (field.value()->text() != value) field.value()->setText(value);
    }
    if (m_prompt && node.kind == Kind::Model) {
        const QString prompt = node.holds.value(QStringLiteral("prompt")).toString();
        m_prompt->setProperty(kShown, prompt);
        if (!(m_prompt->hasFocus() && m_prompt->document()->isModified()) && m_prompt->toPlainText() != prompt) {
            m_prompt->setPlainText(prompt);
            m_prompt->document()->setModified(false);
        }
    }
    if (m_curve && node.kind == Kind::Signal)
        m_curve->setSignal(node.holds.value(QStringLiteral("values")).toArray(), node.holds.value(QStringLiteral("frames")).toInt(1));
    for (auto toggle = m_switches.constBegin(); toggle != m_switches.constEnd(); ++toggle) {
        const QSignalBlocker quiet(toggle.value());
        toggle.value()->setChecked(now.value(toggle.key()) == QLatin1String("true"));
    }
}

void RecipeInspector::addRow(const QString &label, QWidget *control, int labelWidth) {
    auto *row = new QHBoxLayout;
    row->setContentsMargins(0, 0, 0, 0);
    row->setSpacing(space::s8);
    QLabel *name = text(label, tokens::control, ink2);
    name->setFixedWidth(labelWidth > 0 ? labelWidth : kLabelWidth);
    // A name longer than its room is cut, and whole in its tip.
    name->setText(QFontMetricsF(name->font()).elidedText(label, Qt::ElideRight, name->width()));
    if (name->text() != label) name->setToolTip(label);
    row->addWidget(name, 0, Qt::AlignVCenter);
    row->addWidget(control, 1);
    m_column->addLayout(row);
}

void RecipeInspector::addSection(const QString &title) {
    m_column->addSpacing(space::s6);
    QLabel *header = text(title, meta, ink3);
    header->setObjectName(QStringLiteral("graphSection"));
    m_column->addWidget(header);
}

QLabel *RecipeInspector::addNote(const QString &said, QRgb ink) {
    QLabel *note = text(said, tokens::control, ink, true);
    m_column->addWidget(note);
    return note;
}

void RecipeInspector::commit(QObject *field, int line, const QString &key, const QString &now) {
    // By the line the field was made for and what the field itself last
    // showed: a field that is left as its card gives way to another node's
    // still sets its own node, and nothing of the one that took its place.
    if (line <= 0 || now == field->property(kShown).toString()) return;
    field->setProperty(kShown, now);
    if (now.isEmpty()) emit removed(line, key, 0);
    else emit edited(line, key, 0, now);
}

QLineEdit *RecipeInspector::addField(const QString &name, const QString &label, const QString &key, const QString &placeholder) {
    auto *field = new QLineEdit(m_values.value(name));
    field->setProperty(kShown, m_values.value(name));
    field->setObjectName(QStringLiteral("graphField.%1").arg(name));
    field->setAccessibleName(label);
    field->setPlaceholderText(placeholder);
    field->setFont(tokens::font(timecode));
    connect(field, &QLineEdit::editingFinished, this, [this, field, key, line = m_line] {
        commit(field, line, key, field->text().trimmed());
        field->setModified(false);
    });
    m_fields.insert(name, field);
    addRow(label, field);
    return field;
}

void RecipeInspector::rebuild(const graph::Recipe &recipe, const Node &node) {
    startContent();
    m_values = values(node);

    // What it is: its icon, its name, and what it gives.
    auto *head = new QHBoxLayout;
    head->setSpacing(space::s8);
    auto *glyph = new QLabel;
    glyph->setPixmap(icons::pixmap(graph::glyphOf(node), graph::colorOf(node.gives), 16, devicePixelRatioF()));
    glyph->setFixedSize(16, 16);
    head->addWidget(glyph, 0, Qt::AlignVCenter);
    QLabel *title = text(node.title, tokens::title, ink);
    title->setObjectName(QStringLiteral("graphInspectorTitle"));
    head->addWidget(title, 1, Qt::AlignVCenter);
    head->addWidget(text(node.kind == Kind::Part ? QStringLiteral("#%1").arg(node.holds.value(QStringLiteral("id")).toString())
                                                 : graph::nameOf(node.gives),
                         monoSmall, ink3),
                    0, Qt::AlignVCenter);
    m_column->addLayout(head);

    switch (node.kind) {
    case Kind::Part: buildPart(recipe, node); break;
    case Kind::Model: buildModel(recipe, node); break;
    case Kind::Render: buildRender(node); break;
    case Kind::Signal: buildSignal(node); break;
    case Kind::Takes: buildTakes(node); break;
    case Kind::Moves:
        addNote(node.holds.value(QStringLiteral("there")).toBool()
                    ? QStringLiteral("The model is handed where this clip is on each frame of %1: its place as the sequence draws it, so a "
                                     "camera that moves and what hides it are in it. Move the clip, or its path on the picture, to change it.")
                          .arg(node.holds.value(QStringLiteral("sequence")).toString())
                    : node.detail,
                node.holds.value(QStringLiteral("there")).toBool() ? ink2 : status::failed);
        if (node.holds.value(QStringLiteral("there")).toBool()) buildPointing(node.holds.value(QStringLiteral("clip")).toString(), QStringLiteral("moves"));
        break;
    case Kind::Sequence:
        addNote(node.detail, graph::isMissing(node) ? status::failed : ink2);
        addNote(QStringLiteral("A sequence of this project, which the guide is drawn from. Edit it on the timeline."));
        break;
    case Kind::File: {
        // A picture shows; any file says where it is.
        const QString inside = node.holds.value(QStringLiteral("inProject")).toString();
        const QString file = QDir(m_directory).filePath(inside);
        if (node.gives == Gives::Image && !m_directory.isEmpty() && !inside.isEmpty() && QFileInfo(file).isFile()) {
            QImageReader reader(file);
            reader.setAutoTransform(true);
            const QSize size = reader.size();
            const int width = layout::graphInspector - 2 * kPad;
            if (size.isValid()) reader.setScaledSize(size.scaled(width * 2, 320, Qt::KeepAspectRatio));
            if (const QImage image = reader.read(); !image.isNull()) {
                auto *picture = new QLabel;
                picture->setObjectName(QStringLiteral("graphPicture"));
                QPixmap pixmap = QPixmap::fromImage(image.scaled(QSize(width, 160) * devicePixelRatioF(), Qt::KeepAspectRatio, Qt::SmoothTransformation));
                pixmap.setDevicePixelRatio(devicePixelRatioF());
                picture->setPixmap(pixmap);
                picture->setAlignment(Qt::AlignCenter);
                m_column->addWidget(picture);
            }
        }
        addNote(node.holds.value(QStringLiteral("path")).toString(), ink2)->setFont(tokens::font(timecode));
        if (graph::isMissing(node)) addNote(QStringLiteral("This file is not there. The recipe cannot run until it is."), status::failed);
        break;
    }
    case Kind::Cast:
    case Kind::Style: {
        addNote(node.detail, graph::isMissing(node) ? status::failed : ink2);
        QStringList pictures;
        for (const QJsonValue picture : node.holds.value(QStringLiteral("refs")).toArray()) pictures << picture.toString();
        if (!pictures.isEmpty()) addNote(pictures.join(QLatin1Char('\n')))->setFont(tokens::font(monoSmall));
        auto *open = new ui::GhostButton(QStringLiteral("Cast and Styles…"));
        open->setObjectName(QStringLiteral("graphCast"));
        connect(open, &QAbstractButton::clicked, this, &RecipeInspector::castRequested);
        m_column->addWidget(open, 0, Qt::AlignLeft);
        break;
    }
    case Kind::Join:
        addNote(QStringLiteral("Kerf sets the parts it reads end to end into one file: a shot longer than a model makes at once, made a part at a time."));
        addField(QStringLiteral("lap"), QStringLiteral("Lap"), QStringLiteral("lap"), QStringLiteral("0 frames"));
        addNote(QStringLiteral("Lap is how many frames each part after the first shares with the one before it, which are left out: 1 where a "
                               "part goes on from the last frame of the one before."));
        buildReads(recipe, node);
        break;
    case Kind::Run:
        addNote(QStringLiteral("%1 %2").arg(node.holds.value(QStringLiteral("program")).toString(), node.detail), ink2)->setFont(tokens::font(timecode));
        addNote(QStringLiteral("A program the recipe runs, in its own folder. Its line is the recipe's to edit as text."));
        buildReads(recipe, node);
        break;
    }
    if (node.line > 0 && node.kind != Kind::Part) addRemoveStep(node);
    m_column->addStretch(1);
    m_scroll->setWidget(m_content);
    m_content->show();
}

void RecipeInspector::buildModel(const graph::Recipe &recipe, const Node &node) {
    const QJsonObject &holds = node.holds;
    if (const QString about = holds.value(QStringLiteral("about")).toString(); !about.isEmpty()) addNote(about);
    if (graph::isMissing(node)) {
        // Named by the recipe, and by nothing of the person's: it cannot be asked.
        QLabel *missing = addNote(QStringLiteral("No model named %1 is set up on this computer. A model is a program your own models file names.")
                                      .arg(holds.value(QStringLiteral("model")).toString()),
                                  status::request);
        missing->setObjectName(QStringLiteral("graphNotSetUp"));
        auto *models = new ui::GhostButton(QStringLiteral("Models…"));
        models->setObjectName(QStringLiteral("graphModels"));
        connect(models, &QAbstractButton::clicked, this, &RecipeInspector::modelsRequested);
        m_column->addWidget(models, 0, Qt::AlignLeft);
    }
    addSection(QStringLiteral("Prompt"));
    auto *prompt = new PromptEdit;
    prompt->setObjectName(QStringLiteral("graphField.prompt"));
    prompt->setAccessibleName(QStringLiteral("Prompt"));
    prompt->setPlaceholderText(QStringLiteral("What the file should show"));
    prompt->setFont(tokens::font(body));
    prompt->setTabChangesFocus(true);
    prompt->setFixedHeight(88);
    prompt->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    prompt->setProperty("bezelRadius", 6);
    prompt->setProperty(kShown, holds.value(QStringLiteral("prompt")).toString());
    prompt->setPlainText(holds.value(QStringLiteral("prompt")).toString());
    prompt->document()->setModified(false);
    prompt->done = [this, prompt, line = m_line] {
        commit(prompt, line, QStringLiteral("prompt"), prompt->toPlainText().simplified());
        prompt->document()->setModified(false);
    };
    m_prompt = prompt;
    m_column->addWidget(prompt);

    // The seed, and another at a press: the same shot, rolled again.
    auto *seedRow = new QWidget;
    auto *seedLayout = new QHBoxLayout(seedRow);
    seedLayout->setContentsMargins(0, 0, 0, 0);
    seedLayout->setSpacing(space::s4);
    auto *seed = new QLineEdit(m_values.value(QStringLiteral("seed")));
    seed->setProperty(kShown, m_values.value(QStringLiteral("seed")));
    seed->setObjectName(QStringLiteral("graphField.seed"));
    seed->setAccessibleName(QStringLiteral("Seed"));
    seed->setPlaceholderText(QStringLiteral("the model picks one"));
    seed->setFont(tokens::font(timecode));
    connect(seed, &QLineEdit::editingFinished, this, [this, seed, line = m_line] {
        commit(seed, line, QStringLiteral("seed"), seed->text().trimmed());
        seed->setModified(false);
    });
    m_fields.insert(QStringLiteral("seed"), seed);
    seedLayout->addWidget(seed, 1);
    auto *another = new ui::IconButton(QStringLiteral("g-dice"), ui::IconButton::Size::S24);
    another->setObjectName(QStringLiteral("graphSeedRoll"));
    another->setCheckable(false);
    another->setAccessibleName(QStringLiteral("Another seed"));
    another->setTip(QStringLiteral("Another seed"));
    connect(another, &QAbstractButton::clicked, this, [this, seed, line = m_line] {
        // A seed a model's sampler takes: a whole number under 2^31.
        const QString rolled = QString::number(QRandomGenerator::global()->bounded(1u << 31));
        seed->setText(rolled);
        commit(seed, line, QStringLiteral("seed"), rolled);
    });
    seedLayout->addWidget(another);
    addRow(QStringLiteral("Seed"), seedRow);
    const QString longest = holds.contains(QStringLiteral("modelSeconds"))
                                ? QStringLiteral("up to %1").arg(shownValue(holds.value(QStringLiteral("modelSeconds")))) : QStringLiteral("as the model makes");
    addField(QStringLiteral("seconds"), QStringLiteral("Seconds"), QStringLiteral("seconds"), longest);
    addField(QStringLiteral("size"), QStringLiteral("Size"), QStringLiteral("size"),
             holds.value(QStringLiteral("modelSize")).toString(QStringLiteral("the model's own")));
    // A part of a longer shot says where in its sequences' time it starts: its signals and
    // where its clips go are read from there.
    if (!holds.value(QStringLiteral("from")).isNull())
        addField(QStringLiteral("from"), QStringLiteral("From"), QStringLiteral("from"), QStringLiteral("the sequence's start"));

    // The knobs: what the line sets for this model alone, then what the model
    // has said it takes, each as it comes until the line says otherwise.
    const QJsonArray knobs = holds.value(QStringLiteral("knobs")).toArray();
    addSection(QStringLiteral("Knobs"));
    for (const QJsonValue value : knobs) {
        const QJsonObject knob = value.toObject();
        const QString name = knob.value(QStringLiteral("name")).toString();
        const QString comes = shownValue(knob.value(QStringLiteral("default")));
        QLineEdit *field = addField(QStringLiteral("knob.") + name, name, name, comes);
        field->setToolTip(comes.isEmpty() ? QStringLiteral("%1, as the line sets it; emptied, it is taken off the line").arg(name)
                                          : QStringLiteral("%1: %2 as the model comes; emptied, it is that again").arg(name, comes));
    }
    // Any other name goes to the model as it is written.
    auto *more = new QLineEdit;
    more->setObjectName(QStringLiteral("graphField.add"));
    more->setAccessibleName(QStringLiteral("Add a knob"));
    more->setPlaceholderText(QStringLiteral("name=value"));
    more->setFont(tokens::font(timecode));
    connect(more, &QLineEdit::editingFinished, this, [this, more, line = m_line] {
        const QString said = more->text().trimmed();
        const qsizetype equals = said.indexOf(QLatin1Char('='));
        if (equals <= 0 || line <= 0) return;
        const QString name = said.left(equals).trimmed();
        const QString value = said.mid(equals + 1).trimmed();
        more->clear();
        if (!value.isEmpty()) emit edited(line, name, 0, value);
    });
    addRow(QStringLiteral("Add"), more);
    // A model its entry runs with kerf-comfy has a workflow behind it: the
    // ComfyUI graph that is run, which opens to be seen and set.
    if (const QJsonObject runner = holds.value(QStringLiteral("runner")).toObject(); runner.value(QStringLiteral("kind")).toString() == QLatin1String("comfy")) {
        addSection(QStringLiteral("Workflow"));
        const QString own = holds.value(QStringLiteral("workflow")).toString();
        addNote(own.isEmpty() ? QStringLiteral("%1, as it comes with kerf-comfy.").arg(runner.value(QStringLiteral("workflow")).toString())
                              : QStringLiteral("%1, this shot's own copy.").arg(own),
                ink2)
            ->setObjectName(QStringLiteral("graphWorkflowSays"));
        auto *open = new ui::Button(QStringLiteral("Open workflow"), ui::Button::Kind::Secondary);
        open->setObjectName(QStringLiteral("graphOpenWorkflow"));
        open->setTip(QStringLiteral("Every node and input of the ComfyUI graph this model is run with"));
        connect(open, &QAbstractButton::clicked, this, [this, line = m_line] { emit workflowRequested(line); });
        m_column->addWidget(open, 0, Qt::AlignLeft);
    }
    buildReads(recipe, node);
}

void RecipeInspector::rebuildWorkflow() {
    startContent();
    QLabel *title = text(m_workflow.name, tokens::title, ink);
    title->setObjectName(QStringLiteral("graphInspectorTitle"));
    m_column->addWidget(title);
    addNote(QStringLiteral("The ComfyUI workflow model %1 is run with.").arg(m_workflow.model));
    if (!m_workflow.about.isEmpty()) addNote(m_workflow.about, ink2);
    // Where it comes from, and what setting a value in it does.
    QLabel *origin = nullptr;
    if (m_workflow.isOwn()) {
        origin = addNote(QStringLiteral("This shot's own copy, %1, which its line names. It is what this shot is made with; nothing on the "
                                        "ComfyUI server is changed.").arg(m_workflow.file), ink2);
        auto *drop = new ui::GhostButton(QStringLiteral("Use it as it comes"));
        drop->setObjectName(QStringLiteral("graphWorkflowDrop"));
        drop->setTip(QStringLiteral("Take workflow= off the line; the copy stays in the project"));
        connect(drop, &QAbstractButton::clicked, this, [this, line = m_workflow.line] { emit workflowDropped(line); });
        m_column->addWidget(drop, 0, Qt::AlignLeft);
    } else {
        origin = addNote(QStringLiteral("As it comes%1. Set a value in it and this shot keeps a copy with that change, beside its recipe, which its "
                                        "line then runs. Nothing on the ComfyUI server, and nothing of kerf-comfy, is changed.")
                             .arg(m_workflow.origin == QLatin1String("file") ? QStringLiteral(", from the file your models entry names")
                                                                             : QStringLiteral(" with kerf-comfy")),
                         ink2);
    }
    origin->setObjectName(QStringLiteral("graphWorkflowOrigin"));
    if (!m_workflow.blanks.isEmpty()) {
        addSection(QStringLiteral("Blanks the recipe fills"));
        for (auto blank = m_workflow.blanks.constBegin(); blank != m_workflow.blanks.constEnd(); ++blank) {
            QLabel *name = text(QStringLiteral("{%1}").arg(blank.key()), timecode, accentInk);
            m_column->addWidget(name);
            addNote(blankSays(blank.key()))->setProperty("graphBlank", blank.key());
        }
    }
    addNote(QStringLiteral("Choose a node to see its inputs. A blank is set on the recipe's line; any other value is set here."));
    m_column->addStretch(1);
    m_scroll->setWidget(m_content);
    m_content->show();
}

void RecipeInspector::buildPart(const graph::Recipe &recipe, const Node &node) {
    if (!node.detail.isEmpty()) addNote(node.detail)->setFont(tokens::font(monoSmall));
    const QString id = node.holds.value(QStringLiteral("id")).toString();
    const auto said = [](const QString &what, const TypeStyle &style, QRgb ink) { return text(what, style, ink, true); };
    for (const QJsonValue value : node.holds.value(QStringLiteral("inputs")).toArray()) {
        const QJsonObject input = value.toObject();
        const QString name = input.value(QStringLiteral("name")).toString();
        switch (graph::inputKind(input)) {
        case graph::InputKind::Link: {
            // Where it comes from: the graph's own shape, said and not set.
            const QJsonArray link = input.value(QStringLiteral("link")).toArray();
            const Node *from = recipe.node(QStringLiteral("part:%1").arg(link.at(0).toString()));
            addRow(name, said(QStringLiteral("← %1 #%2").arg(from ? from->title : QStringLiteral("a node"), link.at(0).toString()), tokens::control, ink3), kInputLabelWidth);
            break;
        }
        case graph::InputKind::Blank: {
            // The recipe's to fill: marked, with what fills it and what the line gives now.
            QLabel *blank = said(graph::valueText(input.value(QStringLiteral("value"))), timecode, accentInk);
            blank->setObjectName(QStringLiteral("graphBlank.%1").arg(name));
            addRow(name, blank, kInputLabelWidth);
            QStringList says;
            for (const QJsonValue filled : input.value(QStringLiteral("blanks")).toArray()) says << blankSays(filled.toString());
            says.removeAll(QString());
            if (!says.isEmpty()) addNote(says.join(QLatin1Char('\n')));
            break;
        }
        case graph::InputKind::Value: {
            const QJsonValue held = input.value(QStringLiteral("value"));
            const QString field = QStringLiteral("input.") + name;
            if (held.isBool()) {
                auto *toggle = new ui::Switch;
                toggle->setObjectName(QStringLiteral("graphField.%1").arg(field));
                toggle->setAccessibleName(name);
                toggle->setChecked(held.toBool());
                connect(toggle, &QAbstractButton::toggled, this, [this, id, name](bool on) { emit partEdited(id, name, on); });
                m_switches.insert(field, toggle);
                auto *holder = new QWidget;
                auto *row = new QHBoxLayout(holder);
                row->setContentsMargins(0, 0, 0, 0);
                row->addWidget(toggle, 0, Qt::AlignLeft);
                row->addStretch(1);
                addRow(name, holder, kInputLabelWidth);
                break;
            }
            if (!held.isString() && !held.isDouble()) {
                // A list or an object the workflow sets whole: shown, and the file's to change.
                addRow(name, said(graph::valueText(held), timecode, ink2), kInputLabelWidth);
                break;
            }
            // One of the values the server takes there: chosen from them, not typed.
            if (const QJsonArray choices = input.value(QStringLiteral("choices")).toArray(); !choices.isEmpty()) {
                auto *choose = new ui::GhostButton(graph::valueText(held));
                choose->setObjectName(QStringLiteral("graphChoice.%1").arg(name));
                choose->setAccessibleName(name);
                choose->setChevron(true);
                connect(choose, &QAbstractButton::clicked, this, [this, choose, choices, held, id, name] {
                    auto *menu = new QMenu(choose);
                    menu->setObjectName(QStringLiteral("graphChoiceMenu"));
                    menu->setAttribute(Qt::WA_DeleteOnClose);
                    ui::styleMenu(menu);
                    for (const QJsonValue choice : choices) {
                        QAction *action = menu->addAction(graph::valueText(choice));
                        action->setCheckable(true);
                        action->setChecked(choice == held);
                        connect(action, &QAction::triggered, this, [this, id, name, choice, held] {
                            if (choice != held) emit partEdited(id, name, choice);
                        });
                    }
                    ui::popupMenu(menu, choose);
                });
                auto *holder = new QWidget;
                auto *row = new QHBoxLayout(holder);
                row->setContentsMargins(0, 0, 0, 0);
                row->addWidget(choose, 0, Qt::AlignLeft);
                row->addStretch(1);
                addRow(name, holder, kInputLabelWidth);
                break;
            }
            auto *edit = new QLineEdit(m_values.value(field));
            edit->setProperty(kShown, m_values.value(field));
            edit->setObjectName(QStringLiteral("graphField.%1").arg(field));
            edit->setAccessibleName(name);
            edit->setFont(tokens::font(timecode));
            const bool number = held.isDouble();
            // The least and the most the server takes there, where it has said.
            const QJsonValue least = input.value(QStringLiteral("min"));
            const QJsonValue most = input.value(QStringLiteral("max"));
            if (number && (least.isDouble() || most.isDouble()))
                edit->setToolTip(QStringLiteral("%1 to %2").arg(least.isDouble() ? graph::valueText(least) : QStringLiteral("any"),
                                                                 most.isDouble() ? graph::valueText(most) : QStringLiteral("any")));
            connect(edit, &QLineEdit::editingFinished, this, [this, edit, id, name, number, least, most] {
                const QString now = edit->text().trimmed();
                const QString was = edit->property(kShown).toString();
                edit->setModified(false);
                if (now == was) return;
                if (!number) {
                    edit->setProperty(kShown, now);
                    emit partEdited(id, name, now);
                    return;
                }
                // A number stays a number: what does not read as one is put back.
                bool reads = false;
                const double value = now.toDouble(&reads);
                // Nor is one the server does not take there sent to be refused.
                if (!reads || (least.isDouble() && value < least.toDouble()) || (most.isDouble() && value > most.toDouble())) {
                    edit->setText(was);
                    return;
                }
                edit->setProperty(kShown, now);
                emit partEdited(id, name, value);
            });
            m_fields.insert(field, edit);
            addRow(name, edit, kInputLabelWidth);
            break;
        }
        }
    }
}

void RecipeInspector::buildReads(const graph::Recipe &recipe, const Node &node) {
    const QList<Edge> into = recipe.into(node.id);
    if (into.isEmpty()) return;
    addSection(QStringLiteral("Reads"));
    for (const Edge &edge : into) {
        const Node *from = recipe.node(edge.from);
        auto *row = new QWidget;
        auto *layout = new QHBoxLayout(row);
        layout->setContentsMargins(0, 0, 0, 0);
        layout->setSpacing(space::s8);
        auto *dot = new ui::Dot(graph::colorOf(edge.gives));
        layout->addWidget(dot, 0, Qt::AlignVCenter);
        QLabel *port = text(edge.port, timecode, ink2);
        port->setFixedWidth(kLabelWidth - 14);
        layout->addWidget(port, 0, Qt::AlignVCenter);
        layout->addWidget(text(from ? from->title : edge.from, tokens::control, ink), 1, Qt::AlignVCenter);
        // A reading is let go by taking its word off the line; a guide's
        // sequence and what a program needs are not words of that kind.
        if (node.kind == Kind::Model) {
            auto *letGo = new ui::IconButton(QStringLiteral("g-close"), ui::IconButton::Size::S20);
            letGo->setObjectName(QStringLiteral("graphLetGo.%1.%2").arg(edge.port).arg(edge.nth));
            letGo->setCheckable(false);
            letGo->setAccessibleName(QStringLiteral("Let go of %1").arg(edge.port));
            letGo->setTip(QStringLiteral("Take %1 off the line").arg(edge.port));
            const int line = node.line;
            connect(letGo, &QAbstractButton::clicked, this, [this, line, edge] { emit removed(line, edge.port, edge.nth); });
            layout->addWidget(letGo, 0, Qt::AlignVCenter);
        }
        m_column->addWidget(row);
    }
}

void RecipeInspector::buildRender(const Node &node) {
    const QJsonObject &holds = node.holds;
    addNote(QStringLiteral("Kerf draws a sequence of this project into a file a model follows, with no sound."));
    addField(QStringLiteral("sequence"), QStringLiteral("Sequence"), QStringLiteral("sequence"));
    // What of the sequence is drawn: its picture, or a pass of it.
    static const QList<QPair<QString, QString>> passes{
        {QStringLiteral("picture"), QStringLiteral("Picture")}, {QStringLiteral("depth"), QStringLiteral("Depth")},
        {QStringLiteral("relative"), QStringLiteral("Relative depth")}, {QStringLiteral("ids"), QStringLiteral("Ids")},
        {QStringLiteral("normals"), QStringLiteral("Normals")}};
    const QString pass = holds.value(QStringLiteral("pass")).toString(QStringLiteral("picture"));
    QString shown = pass;
    for (const auto &[id, name] : passes)
        if (id == pass) shown = name;
    auto *draws = new ui::GhostButton(shown);
    draws->setObjectName(QStringLiteral("graphPass"));
    draws->setAccessibleName(QStringLiteral("Draws"));
    draws->setChevron(true);
    const int line = node.line;
    connect(draws, &QAbstractButton::clicked, this, [this, draws, pass, line] {
        auto *menu = new QMenu(draws);
        menu->setObjectName(QStringLiteral("graphPassMenu"));
        menu->setAttribute(Qt::WA_DeleteOnClose);
        ui::styleMenu(menu);
        for (const auto &[id, name] : passes) {
            QAction *action = menu->addAction(name);
            action->setObjectName(QStringLiteral("graphPass.%1").arg(id));
            action->setCheckable(true);
            action->setChecked(id == pass);
            connect(action, &QAction::triggered, this, [this, id = id, pass, line] {
                if (id == pass) return;
                // The picture is what a line draws when it names no pass.
                if (id == QLatin1String("picture")) emit removed(line, QStringLiteral("pass"), 0);
                else emit edited(line, QStringLiteral("pass"), 0, id);
            });
        }
        ui::popupMenu(menu, draws);
    });
    auto *holder = new QWidget;
    auto *holderLayout = new QHBoxLayout(holder);
    holderLayout->setContentsMargins(0, 0, 0, 0);
    holderLayout->addWidget(draws, 0, Qt::AlignLeft);
    holderLayout->addStretch(1);
    addRow(QStringLiteral("Draws"), holder);
    addField(QStringLiteral("size"), QStringLiteral("Size"), QStringLiteral("size"), QStringLiteral("the sequence's"));
    addField(QStringLiteral("rate"), QStringLiteral("Rate"), QStringLiteral("rate"), QStringLiteral("the sequence's"));
    addField(QStringLiteral("matte"), QStringLiteral("Matte of"), QStringLiteral("matte"), QStringLiteral("a clip, alone in white"));
    // The part of the sequence it draws, in seconds: a shot longer than a model makes at once is drawn a part at a time.
    addField(QStringLiteral("from"), QStringLiteral("From"), QStringLiteral("from"), QStringLiteral("its start"));
    addField(QStringLiteral("seconds"), QStringLiteral("Seconds"), QStringLiteral("seconds"), QStringLiteral("to its end"));
    // A matte is one clip in white on black: of footage, as much of it as a mask cuts out.
    if (const QString matte = holds.value(QStringLiteral("matte")).toString(); !matte.isEmpty()) buildPointing(matte, QStringLiteral("matte"));
    // The guide as it was drawn for the take that shows: what the model was handed, to look at beside the take.
    const QJsonObject made = holds.value(QStringLiteral("made")).toObject();
    if (const QImage drawn = m_pictures.value(made.value(QStringLiteral("path")).toString()); !drawn.isNull()) {
        addSection(QStringLiteral("As drawn for take %1").arg(made.value(QStringLiteral("take")).toInt()));
        auto *picture = new QLabel;
        picture->setObjectName(QStringLiteral("graphGuidePicture"));
        picture->setAccessibleName(QStringLiteral("The guide as drawn for take %1").arg(made.value(QStringLiteral("take")).toInt()));
        const int width = layout::graphInspector - 2 * kPad;
        QPixmap pixmap = QPixmap::fromImage(drawn.scaled(QSize(width, 160) * devicePixelRatioF(), Qt::KeepAspectRatio, Qt::SmoothTransformation));
        pixmap.setDevicePixelRatio(devicePixelRatioF());
        picture->setPixmap(pixmap);
        m_column->addWidget(picture, 0, Qt::AlignLeft);
    }
}

void RecipeInspector::buildSignal(const Node &node) {
    const QJsonObject &holds = node.holds;
    if (graph::isMissing(node)) {
        addNote(node.detail, status::failed);
        return;
    }
    addNote(QStringLiteral("A number from 0 to 1 over the shot's time. The prompt says what it is of; this says when, and how much."));
    m_curve = new SignalCurve;
    m_curve->setSignal(holds.value(QStringLiteral("values")).toArray(), holds.value(QStringLiteral("frames")).toInt(1));
    const QString clip = holds.value(QStringLiteral("clip")).toString();
    connect(m_curve, &SignalCurve::drawn, this, [this, clip](const QJsonArray &keys) { emit signalDrawn(clip, keys); });
    m_column->addWidget(m_curve);
    addNote(QStringLiteral("%1. Drag across it to draw it anew; it is the clip's value in the project, and is undone as any edit.").arg(node.detail));
    // Its moments read from a sound of its sequence: 1 for a few frames from where each sound starts.
    const QJsonArray sounds = holds.value(QStringLiteral("sounds")).toArray();
    if (sounds.isEmpty()) return;
    auto *from = new ui::GhostButton(QStringLiteral("From a sound"));
    from->setObjectName(QStringLiteral("graphSignalFromSound"));
    from->setChevron(true);
    from->setTip(QStringLiteral("Set it to 1 for three frames from each place a sound of its sequence starts, the beats of a track or a ball on a floor, and 0 between"));
    connect(from, &QAbstractButton::clicked, this, [this, from, sounds, clip] {
        auto *menu = new QMenu(from);
        menu->setObjectName(QStringLiteral("graphSoundMenu"));
        menu->setAttribute(Qt::WA_DeleteOnClose);
        ui::styleMenu(menu);
        ui::addMenuHeader(menu, QStringLiteral("Where this sound's sounds start"));
        for (const QJsonValue value : sounds) {
            const QJsonObject sound = value.toObject();
            const QString of = sound.value(QStringLiteral("clip")).toString();
            QAction *action = menu->addAction(sound.value(QStringLiteral("name")).toString(of));
            action->setObjectName(QStringLiteral("graphSound.%1").arg(of));
            connect(action, &QAction::triggered, this, [this, clip, of] { emit signalFromSound(clip, of); });
        }
        ui::popupMenu(menu, from);
    });
    m_column->addWidget(from, 0, Qt::AlignLeft);
}

void RecipeInspector::buildPointing(const QString &clip, const QString &of) {
    const bool moves = of == QLatin1String("moves");
    const bool mine = m_pointing.clip == clip && m_pointing.of == of;
    const QString state = mine ? m_pointing.state : QString();
    addSection(QStringLiteral("From footage"));
    if (mine && !m_pointing.says.isEmpty()) {
        QLabel *says = addNote(m_pointing.says, m_pointing.failed ? status::failed : ink2);
        says->setObjectName(QStringLiteral("graphFollowSays"));
    }
    if (state == QLatin1String("pointing") || state == QLatin1String("following")) {
        const bool following = state == QLatin1String("following");
        auto *picture = new PointPicture(!moves);
        picture->setObjectName(QStringLiteral("graphPointPicture"));
        picture->setAccessibleName(moves ? QStringLiteral("The clip's first frame: press on what it should follow")
                                         : QStringLiteral("The clip's first frame: press on what to cut out"));
        picture->setPicture(m_pointing.picture);
        picture->setPoints(m_pointing.on, m_pointing.off);
        picture->setEnabled(!following);
        m_column->addWidget(picture);
        const auto follow = [this, clip, of] {
            if (m_pointing.on.isEmpty()) return;
            m_pointing.state = QStringLiteral("following");
            m_pointing.says.clear();
            m_pointing.failed = false;
            emit shotFollowRequested(clip, of, PointPicture::said(m_pointing.on), PointPicture::said(m_pointing.off));
        };
        if (following) {
            addNote(moves ? QStringLiteral("Following it through the clip…") : QStringLiteral("Cutting it out through the clip…"))
                ->setObjectName(QStringLiteral("graphFollowing"));
        } else if (moves) {
            addNote(QStringLiteral("Press on what the clip should follow: a corner or a mark, or a thing that stands out from what is behind it. "
                                   "The clip itself is not in this picture."));
            connect(picture, &PointPicture::pointed, this, [this, picture, follow] {
                m_pointing.on = picture->on();
                follow();
            });
        } else {
            addNote(QStringLiteral("Press on what to cut out, in as many places as tell it from the rest; hold Alt, or use the other button, on "
                                   "what is taken for it and is not it."));
            auto *cut = new ui::GhostButton(QStringLiteral("Cut out"));
            cut->setObjectName(QStringLiteral("graphCutOut"));
            cut->setTip(QStringLiteral("Mask the clip to what the points are on, through the whole clip: its matte is then that thing in white"));
            cut->setEnabled(!m_pointing.on.isEmpty());
            connect(picture, &PointPicture::pointed, this, [this, picture, cut] {
                m_pointing.on = picture->on();
                m_pointing.off = picture->off();
                cut->setEnabled(!m_pointing.on.isEmpty());
            });
            connect(cut, &QAbstractButton::clicked, this, follow);
            m_column->addWidget(cut, 0, Qt::AlignLeft);
        }
        return;
    }
    if (state == QLatin1String("drawing")) {
        addNote(QStringLiteral("Drawing the clip's first frame…"))->setObjectName(QStringLiteral("graphFollowing"));
        return;
    }
    auto *ask = new ui::GhostButton(moves ? QStringLiteral("Follow the picture") : QStringLiteral("Cut out of the picture"));
    ask->setObjectName(moves ? QStringLiteral("graphFollowPicture") : QStringLiteral("graphCutPicture"));
    ask->setTip(moves ? QStringLiteral("Point at something in the footage under this clip, and have the clip follow it for as long as it runs: its x and y, keyed")
                      : QStringLiteral("Point at a thing this clip shows, and mask the clip to it as it moves and changes shape: the matte is then that thing"));
    connect(ask, &QAbstractButton::clicked, this, [this, clip, of] {
        m_pointing = Pointing{};
        m_pointing.clip = clip;
        m_pointing.of = of;
        m_pointing.state = QStringLiteral("drawing");
        emit shotPictureRequested(clip, of);
    });
    m_column->addWidget(ask, 0, Qt::AlignLeft);
}

void RecipeInspector::addRemoveStep(const Node &node) {
    auto *remove = new ui::GhostButton(QStringLiteral("Delete step"));
    remove->setObjectName(QStringLiteral("graphDeleteStep"));
    remove->setTip(QStringLiteral("Take line %1 out of the recipe. A step whose file a later one reads, and the last, stay until that is changed").arg(node.line));
    connect(remove, &QAbstractButton::clicked, this, [this, line = node.line] { emit stepRemoveRequested(line); });
    m_column->addWidget(remove, 0, Qt::AlignLeft);
}

void RecipeInspector::buildTakes(const Node &node) {
    const QJsonObject &holds = node.holds;
    const QJsonArray takes = holds.value(QStringLiteral("takes")).toArray();
    if (const QString wants = holds.value(QStringLiteral("wants")).toString(); !wants.isEmpty())
        addNote(QStringLiteral("Wants making: %1.").arg(wants), status::request)->setObjectName(QStringLiteral("graphWants"));
    if (takes.isEmpty()) {
        addNote(QStringLiteral("No take yet. Make runs the recipe and writes the first; every take after it is kept beside it."));
        return;
    }
    addSection(QStringLiteral("Takes"));
    for (const QJsonValue value : takes) {
        const QJsonObject take = value.toObject();
        const int number = take.value(QStringLiteral("number")).toInt();
        const bool chosen = take.value(QStringLiteral("chosen")).toBool();
        // What there is to say of it: how it holds beside its guide once that is read, what
        // has changed since it was made, or which roll it is.
        QStringList changed;
        for (const QJsonValue what : take.value(QStringLiteral("changed")).toArray()) changed << what.toString();
        const int rolled = take.value(QStringLiteral("rolled")).toInt();
        const QJsonObject reading = m_readings.value(number);
        QStringList says;
        QStringList tip;
        if (!reading.isEmpty()) {
            says << QStringLiteral("%1 of its guide%2").arg(reading.value(QStringLiteral("overlap")).toDouble(), 0, 'f', 2)
                        .arg(number == m_best ? QStringLiteral(", the closest") : QString());
            for (const QJsonValue value : reading.value(QStringLiteral("clips")).toArray()) {
                const QJsonObject clip = value.toObject();
                const QString name = clip.value(QStringLiteral("clip")).toString();
                tip << (clip.contains(QStringLiteral("note"))
                            ? QStringLiteral("%1: %2").arg(name, clip.value(QStringLiteral("note")).toString())
                            : QStringLiteral("%1: %2 of where it was asked for, followed on %3 of the %4 frames the guide shows it on")
                                  .arg(name).arg(clip.value(QStringLiteral("overlap")).toDouble(), 0, 'f', 2)
                                  .arg(clip.value(QStringLiteral("frames")).toInt()).arg(clip.value(QStringLiteral("shown")).toInt()));
            }
        }
        if (!changed.isEmpty()) says << QStringLiteral("%1 changed since").arg(changed.join(QStringLiteral(", ")));
        else if (rolled > 0) says << QStringLiteral("roll %1").arg(rolled);
        const QString path = take.value(QStringLiteral("path")).toString();
        auto *row = new TakeRow(number, says.join(QStringLiteral(" · ")), chosen, m_pictures.value(path));
        row->setEnabled(!m_making && !chosen);
        tip.prepend(chosen ? QStringLiteral("The shot is this take")
                           : QStringLiteral("Have the shot be take %1 again; its clips keep their timing, and it is undone as any edit").arg(number));
        row->setTip(tip.join(QLatin1Char('\n')));
        connect(row, &QAbstractButton::clicked, this, [this, number, path] { emit takeChosen(number, path); });
        m_takeRows << row;
        m_column->addWidget(row);
    }
    // The take the shot is, made from a recipe that has changed since: back to that one.
    for (const QJsonValue value : takes) {
        const QJsonObject take = value.toObject();
        if (!take.value(QStringLiteral("chosen")).toBool() || !take.value(QStringLiteral("kept")).toBool() ||
            take.value(QStringLiteral("changed")).toArray().isEmpty())
            continue;
        const int number = take.value(QStringLiteral("number")).toInt();
        auto *restore = new ui::Button(QStringLiteral("Restore take %1's recipe").arg(number), ui::Button::Kind::Secondary);
        restore->setObjectName(QStringLiteral("graphRestoreTake"));
        restore->setEnabled(!m_making);
        restore->setTip(QStringLiteral("Write the recipe back as take %1 was made from it: its words, its seeds, and the workflow it named. "
                                       "The sequences and signals it was made from are the project's, and come back by undoing there").arg(number));
        connect(restore, &QAbstractButton::clicked, this, [this, number] { emit takeRestoreRequested(number); });
        m_column->addWidget(restore, 0, Qt::AlignLeft);
    }
    // Held to the guide: where each take has the shot's subject beside where it was asked for.
    const bool reading = m_readState == QLatin1String("reading");
    auto *hold = new ui::Button(reading ? QStringLiteral("Reading…") : m_readings.isEmpty() ? QStringLiteral("Hold to guide") : QStringLiteral("Hold to guide again"),
                                ui::Button::Kind::Secondary);
    hold->setObjectName(QStringLiteral("graphHoldTakes"));
    hold->setEnabled(!reading);
    hold->setTip(QStringLiteral("Read where each take has the shot's subject beside where its guide asked for it. No model is run"));
    connect(hold, &QAbstractButton::clicked, this, [this] { emit takesReadRequested(); });
    m_column->addWidget(hold, 0, Qt::AlignLeft);
    if (reading)
        addNote(QStringLiteral("Drawing the guide and following the subject through each take. A take is a few seconds."))->setObjectName(QStringLiteral("graphHoldSays"));
    else if (m_readState == QLatin1String("read"))
        addNote(QStringLiteral("1 is a take that has the subject where and as large as its guide asked on every frame, 0 one that has none of it. "
                               "Hover a take for each subject; one followed on few frames says little."))
            ->setObjectName(QStringLiteral("graphHoldSays"));
    else if (!m_readState.isEmpty())
        addNote(m_readState, status::failed)->setObjectName(QStringLiteral("graphHoldSays"));
    addNote(QStringLiteral("Every take is kept. Make again writes the next beside these."));
}

} // namespace kerf
