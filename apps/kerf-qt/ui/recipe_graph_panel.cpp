// The panel a shot's graph opens in (recipe_graph_panel.h).

#include "recipe_graph_panel.h"
#include "components.h"
#include "icons.h"
#include "recipe_inspector.h"

#include <QAction>
#include <QEvent>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QLabel>
#include <QMenu>
#include <QPainter>
#include <QPainterPath>
#include <QResizeEvent>
#include <QVBoxLayout>

#include <algorithm>

namespace kerf {

using namespace tokens;
using graph::Kind;

namespace {

void setInk(QLabel *label, QRgb ink) {
    QPalette palette = label->palette();
    palette.setColor(QPalette::WindowText, color(ink));
    label->setPalette(palette);
}

} // namespace

RecipeGraphPanel::RecipeGraphPanel(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("graphPanel"));
    setAccessibleName(QStringLiteral("Graph"));
    auto *column = new QVBoxLayout(this);
    column->setContentsMargins(1, 1, 1, 1);
    column->setSpacing(0);

    // The head: the shot, how it stands, and its making.
    auto *head = new QWidget;
    head->setObjectName(QStringLiteral("graphHead"));
    head->setFixedHeight(layout::graphBar);
    auto *row = new QHBoxLayout(head);
    row->setContentsMargins(space::s16, 0, space::s12, 0);
    row->setSpacing(space::s8);
    auto *glyph = new QLabel;
    glyph->setPixmap(icons::pixmap(QStringLiteral("g-graph"), color(ink2), 16, devicePixelRatioF()));
    glyph->setFixedSize(16, 16);
    row->addWidget(glyph, 0, Qt::AlignVCenter);
    m_title = new QLabel;
    m_title->setObjectName(QStringLiteral("graphTitle"));
    m_title->setFont(tokens::font(title));
    setInk(m_title, ink);
    row->addWidget(m_title, 0, Qt::AlignVCenter);
    m_path = new QLabel;
    m_path->setObjectName(QStringLiteral("graphPath"));
    m_path->setFont(tokens::font(monoSmall));
    setInk(m_path, ink3);
    row->addWidget(m_path, 0, Qt::AlignVCenter);
    row->addStretch(1);
    m_status = new QLabel;
    m_status->setObjectName(QStringLiteral("graphStatus"));
    m_status->setFont(tokens::font(control));
    m_status->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
    m_status->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    row->addWidget(m_status, 2, Qt::AlignVCenter);
    m_several = new ui::GhostButton(QStringLiteral("Several"));
    m_several->setObjectName(QStringLiteral("graphSeveral"));
    m_several->setChevron(true);
    m_several->setControlHeight(height::h28);
    connect(m_several, &QAbstractButton::clicked, this, [this] {
        // Several takes of the same shot to choose among, each with its seeds one on.
        auto *menu = new QMenu(m_several);
        menu->setObjectName(QStringLiteral("graphSeveralMenu"));
        menu->setAttribute(Qt::WA_DeleteOnClose);
        ui::styleMenu(menu);
        ui::addMenuHeader(menu, QStringLiteral("Each seed one on from the last"));
        for (const int rolls : {2, 3, 4}) {
            QAction *roll = menu->addAction(QStringLiteral("%1 Takes…").arg(rolls));
            roll->setObjectName(QStringLiteral("graphMake%1").arg(rolls));
            connect(roll, &QAction::triggered, this, [this, rolls] { emit makeRequested(rolls); });
        }
        ui::popupMenu(menu, m_several);
    });
    row->addWidget(m_several, 0, Qt::AlignVCenter);
    m_stop = new ui::Button(QStringLiteral("Stop"), ui::Button::Kind::Secondary);
    m_stop->setObjectName(QStringLiteral("graphStop"));
    m_stop->setFixedHeight(height::h28);
    connect(m_stop, &QAbstractButton::clicked, this, &RecipeGraphPanel::stopRequested);
    row->addWidget(m_stop, 0, Qt::AlignVCenter);
    m_make = new ui::Button(QStringLiteral("Make"), ui::Button::Kind::Primary);
    m_make->setObjectName(QStringLiteral("graphMake"));
    m_make->setFixedHeight(height::h28);
    connect(m_make, &QAbstractButton::clicked, this, [this] { emit makeRequested(1); });
    row->addWidget(m_make, 0, Qt::AlignVCenter);
    m_close = new ui::IconButton(QStringLiteral("g-close"), ui::IconButton::Size::S28);
    m_close->setObjectName(QStringLiteral("graphClose"));
    m_close->setCheckable(false);
    m_close->setAccessibleName(QStringLiteral("Close the graph"));
    m_close->setTip(QStringLiteral("Close"), QStringLiteral("Esc"));
    connect(m_close, &QAbstractButton::clicked, this, &RecipeGraphPanel::closeRequested);
    row->addWidget(m_close, 0, Qt::AlignVCenter);
    column->addWidget(head);

    // Under it, the canvas, with the node's card over its right.
    m_body = new QWidget;
    m_body->setObjectName(QStringLiteral("graphBody"));
    auto *bodyLayout = new QVBoxLayout(m_body);
    bodyLayout->setContentsMargins(0, 1, 0, 0);
    m_view = new RecipeGraphView;
    m_view->setFootRadius(radius::panel - 1);
    bodyLayout->addWidget(m_view);
    column->addWidget(m_body, 1);
    m_inspector = new RecipeInspector(m_view);
    // The cards are fitted beside the card that says what one holds, so
    // choosing a node moves none of them.
    m_view->setCovered(layout::graphInspector + 2 * layout::graphPad);
    m_view->installEventFilter(this);

    connect(m_view, &RecipeGraphView::selectionChanged, this, [this](const QString &id) {
        m_inspector->setNode(m_view->recipe(), id);
        placeInspector();
    });
    connect(m_view, &RecipeGraphView::wired, this, &RecipeGraphPanel::wire);
    connect(m_inspector, &RecipeInspector::contentChanged, this, &RecipeGraphPanel::placeInspector);
    connect(m_inspector, &RecipeInspector::edited, this, &RecipeGraphPanel::edited);
    connect(m_inspector, &RecipeInspector::removed, this, &RecipeGraphPanel::removed);
    connect(m_inspector, &RecipeInspector::signalDrawn, this, &RecipeGraphPanel::signalDrawn);
    connect(m_inspector, &RecipeInspector::takeChosen, this, &RecipeGraphPanel::takeChosen);
    connect(m_inspector, &RecipeInspector::castRequested, this, &RecipeGraphPanel::castRequested);
    connect(m_inspector, &RecipeInspector::modelsRequested, this, &RecipeGraphPanel::modelsRequested);
    refreshHead();
}

void RecipeGraphPanel::setProjectDirectory(const QString &directory) { m_inspector->setProjectDirectory(directory); }

const graph::Recipe &RecipeGraphPanel::recipe() const { return m_view->recipe(); }

void RecipeGraphPanel::setRecipe(const graph::Recipe &recipe) {
    m_view->setRecipe(recipe);
    m_inspector->setNode(m_view->recipe(), m_view->selected());
    placeInspector();
    refreshHead();
}

void RecipeGraphPanel::setMaking(bool making, int take, qint64 since, const QString &line) {
    m_making = making;
    m_makingTake = take;
    m_makingLine = line;
    m_view->setMaking(making, take, since);
    m_inspector->setMaking(making);
    refreshHead();
}

void RecipeGraphPanel::setCanMake(bool can, const QString &why) {
    m_canMake = can;
    m_cannotWhy = why;
    refreshHead();
}

void RecipeGraphPanel::refreshHead() {
    const graph::Recipe &recipe = m_view->recipe();
    m_title->setText(recipe.name);
    m_path->setText(recipe.recipe);
    const graph::Node *takes = recipe.node(QStringLiteral("takes"));
    const bool has = takes && !takes->holds.value(QStringLiteral("takes")).toArray().isEmpty();
    const QString wants = takes ? takes->holds.value(QStringLiteral("wants")).toString() : QString();
    // How the shot stands, the most pressing thing first.
    QString said;
    QRgb ink = ink3;
    if (m_making) {
        said = m_makingLine.isEmpty() ? QStringLiteral("Making take %1…").arg(m_makingTake) : QStringLiteral("Making take %1: %2").arg(m_makingTake).arg(m_makingLine);
        ink = tokens::activity::gpu;
    } else if (!recipe.error.isEmpty()) {
        said = QStringLiteral("The recipe does not read");
        ink = status::failed;
    } else if (!recipe.cannot.isEmpty()) {
        said = QStringLiteral("Cannot be made as things stand");
        ink = status::failed;
    } else if (!wants.isEmpty()) {
        said = QStringLiteral("Wants making: %1").arg(wants);
        ink = status::request;
    } else if (takes) {
        said = takes->detail;
    }
    m_status->setText(said);
    m_status->setToolTip(!recipe.error.isEmpty() ? recipe.error : !recipe.cannot.isEmpty() ? recipe.cannot : said);
    setInk(m_status, ink);
    m_make->setText(has ? QStringLiteral("Make again") : QStringLiteral("Make"));
    // A kit button is as wide as what it says when it is made: this one says another thing later.
    m_make->setFixedSize(m_make->sizeHint().width(), height::h28);
    const bool can = m_canMake && recipe.isValid() && recipe.error.isEmpty();
    const QString why = !m_canMake ? m_cannotWhy : !recipe.error.isEmpty() ? QStringLiteral("The recipe does not read") : QString();
    m_make->setVisible(!m_making);
    m_several->setVisible(!m_making);
    m_stop->setVisible(m_making);
    m_make->setEnabled(can);
    m_several->setEnabled(can);
    m_make->setTip(can ? QStringLiteral("Run the recipe and write a new take; what it runs is shown first") : why);
    m_several->setTip(can ? QStringLiteral("Make two to four takes to choose among") : why);
    m_stop->setTip(QStringLiteral("Stop making: its step is asked to end, and no take is kept"));
}

void RecipeGraphPanel::read(const graph::Node &to, const QString &port, const QString &value) {
    // A name a line gives once is replaced; one it may give again is one more.
    static const QStringList several{QStringLiteral("ref"), QStringLiteral("signal"), QStringLiteral("moves"), QStringLiteral("cast"),
                                     QStringLiteral("style")};
    int nth = 0;
    if (several.contains(port))
        for (const graph::Edge &edge : m_view->recipe().into(to.id)) nth += edge.port == port ? 1 : 0;
    emit edited(to.line, port, nth, value);
}

void RecipeGraphPanel::wire(const QString &fromId, const QString &toId, const QPoint &at) {
    const graph::Recipe &recipe = m_view->recipe();
    const graph::Node *from = recipe.node(fromId);
    const graph::Node *to = recipe.node(toId);
    if (!from || !to) return;
    const QStringList ports = graph::portsFor(recipe, *from, *to);
    const QString value = graph::valueOf(*from);
    if (ports.isEmpty() || value.isEmpty()) return;
    if (ports.size() == 1) {
        read(*to, ports.first(), value);
        return;
    }
    // More than one name would read it: which is the person's to say.
    auto *menu = new QMenu(this);
    menu->setObjectName(QStringLiteral("graphPortMenu"));
    menu->setAttribute(Qt::WA_DeleteOnClose);
    ui::styleMenu(menu);
    ui::addMenuHeader(menu, QStringLiteral("%1 reads %2 as").arg(to->title, from->title));
    const QJsonArray reads = to->holds.value(QStringLiteral("reads")).toArray();
    for (const QString &port : ports) {
        QAction *action = menu->addAction(port);
        action->setObjectName(QStringLiteral("graphPort.%1").arg(port));
        // What the model has said it has no use for is still the line's to say, and is said so.
        const bool unread = to->holds.contains(QStringLiteral("reads")) && !reads.contains(port);
        ui::setMenuKey(action, unread ? QStringLiteral("the model has no use for it") : graph::portSays(port));
        connect(action, &QAction::triggered, this, [this, toId, port, value] {
            if (const graph::Node *to = m_view->recipe().node(toId)) read(*to, port, value);
        });
    }
    menu->popup(at);
}

void RecipeGraphPanel::placeInspector() {
    const int most = std::max(0, m_view->height() - 2 * layout::graphPad);
    m_inspector->resize(layout::graphInspector, m_inspector->height());
    const int height = std::max(48, m_inspector->heightFor(most));
    m_inspector->setGeometry(m_view->width() - layout::graphPad - layout::graphInspector, layout::graphPad, layout::graphInspector, height);
    m_inspector->raise();
    m_inspector->show();
}

void RecipeGraphPanel::resizeEvent(QResizeEvent *event) {
    QWidget::resizeEvent(event);
    placeInspector();
}

bool RecipeGraphPanel::eventFilter(QObject *watched, QEvent *event) {
    if (watched == m_view && event->type() == QEvent::Resize) placeInspector();
    return QWidget::eventFilter(watched, event);
}

void RecipeGraphPanel::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const QRectF card = QRectF(rect());
    // The head in the window's chrome, over the canvas, in one rounded card.
    QPainterPath clip;
    clip.addRoundedRect(card, radius::panel, radius::panel);
    p.setClipPath(clip);
    p.fillRect(card, color(tokens::nodes::canvas));
    p.fillRect(QRectF(card.left(), card.top(), card.width(), layout::graphBar + 1), color(chrome));
    p.fillRect(QRectF(card.left(), card.top() + layout::graphBar + 1, card.width(), 1), color(line));
    p.setClipping(false);
    ui::paintInsetLine(p, card, radius::panel, color(line2));
}

} // namespace kerf
