// What the node chosen in a shot's graph holds, said and set (recipe_inspector.h).

#include "recipe_inspector.h"
#include "components.h"
#include "icons.h"
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
// The card's own padding, and how wide a row's name is.
constexpr int kPad = 14;
constexpr int kLabelWidth = 78;

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

// One of the shot's takes: its number, what there is to say of it, and a
// tick on the one the shot is. A press has the shot be it.
class TakeRow final : public ui::KitButton {
public:
    TakeRow(int number, const QString &said, bool chosen, QWidget *parent = nullptr)
        : KitButton(parent), m_number(number), m_said(said), m_chosen(chosen) {
        setObjectName(QStringLiteral("graphTake%1").arg(number));
        setAccessibleName(QStringLiteral("Take %1").arg(number));
        setFixedHeight(height::h28);
        setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
        setCheckable(true);
        setChecked(chosen);
    }

protected:
    // Which take the shot is is the document's to say, not a press's.
    void nextCheckState() override {}
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing, true);
        const QRectF row = QRectF(rect());
        if (isEnabled() && hoverProgress() > 0) {
            p.setPen(Qt::NoPen);
            p.setBrush(ui::mix(Qt::transparent, color(popupHover), hoverProgress()));
            p.drawRoundedRect(row, radius::card, radius::card);
        }
        if (focusVisible()) ui::paintFocusRing(p, row.adjusted(2, 2, -2, -2), radius::control);
        p.setFont(tokens::font(m_chosen ? bodyName : body));
        p.setPen(color(isEnabled() || m_chosen ? ink : dis));
        const QString name = QStringLiteral("Take %1").arg(m_number);
        p.drawText(row.adjusted(8, 0, -8, 0), Qt::AlignLeft | Qt::AlignVCenter, name);
        const qreal nameWidth = QFontMetricsF(p.font()).horizontalAdvance(name);
        const qreal end = m_chosen ? row.right() - 8 - 14 - 6 : row.right() - 8;
        const QFont quiet = tokens::font(control);
        p.setFont(quiet);
        p.setPen(color(ink3));
        const qreal room = end - (row.left() + 8 + nameWidth + 12);
        if (room > 24)
            p.drawText(QRectF(end - room, row.top(), room, row.height()), Qt::AlignRight | Qt::AlignVCenter,
                       QFontMetricsF(quiet).elidedText(m_said, Qt::ElideRight, room));
        if (m_chosen) icons::paint(p, QStringLiteral("g-check"), QRectF(row.right() - 8 - 14, row.center().y() - 7, 14, 14), color(accentInk), 14);
    }

private:
    int m_number;
    QString m_said;
    bool m_chosen;
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
    for (const QJsonValue value : node.holds.value(QStringLiteral("takes")).toArray()) {
        const QJsonObject take = value.toObject();
        parts << QStringLiteral("%1%2%3").arg(take.value(QStringLiteral("number")).toInt()).arg(take.value(QStringLiteral("chosen")).toBool() ? '*' : ' ')
                     .arg(take.value(QStringLiteral("changed")).toArray().size());
    }
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
        shown.insert(QStringLiteral("size"), holds.value(QStringLiteral("size")).toString());
        for (const QJsonValue value : holds.value(QStringLiteral("knobs")).toArray()) {
            const QJsonObject knob = value.toObject();
            shown.insert(QStringLiteral("knob.") + knob.value(QStringLiteral("name")).toString(), knob.value(QStringLiteral("value")).toString());
        }
    } else if (node.kind == Kind::Render) {
        shown.insert(QStringLiteral("sequence"), holds.value(QStringLiteral("sequence")).toString());
        shown.insert(QStringLiteral("size"), holds.value(QStringLiteral("size")).toString());
        shown.insert(QStringLiteral("rate"), holds.value(QStringLiteral("rate")).toString());
        shown.insert(QStringLiteral("matte"), holds.value(QStringLiteral("matte")).toString());
    }
    return shown;
}

void RecipeInspector::setNode(const graph::Recipe &recipe, const QString &id) {
    const Node *node = recipe.node(id);
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

void RecipeInspector::setMaking(bool making) {
    if (making == m_making) return;
    m_making = making;
    for (QWidget *row : std::as_const(m_takeRows)) row->setEnabled(!making && !static_cast<QAbstractButton *>(row)->isChecked());
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
}

void RecipeInspector::addRow(const QString &label, QWidget *control) {
    auto *row = new QHBoxLayout;
    row->setContentsMargins(0, 0, 0, 0);
    row->setSpacing(space::s8);
    QLabel *name = text(label, tokens::control, ink2);
    name->setFixedWidth(kLabelWidth);
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
    head->addWidget(text(graph::nameOf(node.gives), monoSmall, ink3), 0, Qt::AlignVCenter);
    m_column->addLayout(head);

    switch (node.kind) {
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
    case Kind::Run:
        addNote(QStringLiteral("%1 %2").arg(node.holds.value(QStringLiteral("program")).toString(), node.detail), ink2)->setFont(tokens::font(timecode));
        addNote(QStringLiteral("A program the recipe runs, in its own folder. Its line is the recipe's to edit as text."));
        buildReads(recipe, node);
        break;
    }
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
    buildReads(recipe, node);
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
        // What there is to say of it: what has changed since it was made, or which roll it is.
        QStringList changed;
        for (const QJsonValue what : take.value(QStringLiteral("changed")).toArray()) changed << what.toString();
        const int rolled = take.value(QStringLiteral("rolled")).toInt();
        const QString said = !changed.isEmpty() ? QStringLiteral("%1 changed since").arg(changed.join(QStringLiteral(", ")))
                             : rolled > 0 ? QStringLiteral("roll %1").arg(rolled) : QString();
        auto *row = new TakeRow(number, said, chosen);
        row->setEnabled(!m_making && !chosen);
        row->setTip(chosen ? QStringLiteral("The shot is this take")
                           : QStringLiteral("Have the shot be take %1 again; its clips keep their timing, and it is undone as any edit").arg(number));
        const QString path = take.value(QStringLiteral("path")).toString();
        connect(row, &QAbstractButton::clicked, this, [this, number, path] { emit takeChosen(number, path); });
        m_takeRows << row;
        m_column->addWidget(row);
    }
    addNote(QStringLiteral("Every take is kept. Make again writes the next beside these."));
}

} // namespace kerf
